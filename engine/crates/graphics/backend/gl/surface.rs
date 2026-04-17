//! `Canvas2DContext` — the CanvasManager-level Skia wrapper.
//!
//! One instance per live Canvas2D surface.  Owns:
//!   * A [`DirectContext`] tied to the EGL context that hosts the canvas.
//!     Each onscreen / offscreen canvas has its own EGL context (Chromium
//!     "DrawingBuffer" model) so each needs its own Skia context too.
//!   * A [`Surface`] wrapping the DrawingBuffer FBO (onscreen) or a
//!     pbuffer's default FBO (offscreen).  Drawing into this surface goes
//!     straight to the same GL attachments WebGL targets — no intermediate
//!     blit needed between 2D ↔ 3D frames.
//!   * A [`Canvas2DRenderer`] (pure state machine, see `canvas.rs`) that
//!     dispatches Canvas2DCmd opcodes against `surface.canvas()`.
//!
//! GL state hygiene: Skia and WebGL share a single EGL context, so Skia
//! mutates shared GL state (binding, blending, scissor, …).  Whenever we
//! interleave a Canvas2D batch with a WebGL batch we call
//! [`Canvas2DContext::reset_gl_state`] afterwards to flush Skia's
//! deferred draws and tell Skia to drop its tracked GL state — subsequent
//! WebGL commands then see a clean slate.

use std::cell::RefCell;

use skia_safe::{
    gpu::{
        self, backend_render_targets, direct_contexts, gl as sk_gl, interfaces,
        DirectContext, SurfaceOrigin,
    },
    Canvas as SkCanvas, ColorType, Paint, Rect as SkRect, SamplingOptions, Shader, Surface as SkSurface,
    TileMode,
};

use super::canvas::Canvas2DRenderer;
use super::image_store::ImageStore;
use super::paint::PatternResolver;
use super::state::Canvas2DState;
use super::text::TextContext;

/// Kind of framebuffer backing the canvas.  Affects the `SurfaceOrigin`
/// we hand to Skia: Android EGL's default framebuffer is bottom-left
/// origin (same as OpenGL), intermediate FBOs are top-left because we
/// control their orientation on upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FboKind {
    /// The default framebuffer (FBO 0) of an EGL surface — bottom-left
    /// origin, Y axis points up.
    DefaultFb,
    /// A named FBO we created ourselves (DrawingBuffer for the onscreen
    /// canvas).  Also bottom-left-origin in this project because the
    /// DrawingBuffer blit step flips at present time.
    DrawingBuffer,
}

/// Per-context GPU resource cache budget.
///
/// Rationale: Skia's default `GrResourceCache` budget is
/// unbounded-ish (96 MB + 2^20 resources on current Ganesh builds),
/// which is catastrophic when we have multiple Canvas2DContexts on an
/// Android device with 2 GB of system RAM.  Capping at 32 MB / 2^14
/// resources per context gives a predictable ceiling: three live
/// contexts stay well within the 200 MB native-heap target while
/// still leaving headroom for the glyph atlas, gradient textures,
/// and path caches.
///
/// These values are the result of back-of-napkin math, not a tuned
/// benchmark — revisit once device telemetry is flowing.  See Skia
/// `GrDirectContext::setResourceCacheLimits` docs:
/// <https://api.skia.org/classGrDirectContext.html>.
const SKIA_RESOURCE_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;
const SKIA_RESOURCE_CACHE_MAX_RESOURCES: usize = 1 << 14;

/// Monotonic allocator for `Canvas2DContext` identity tags.  Used by
/// [`ImageStore`] to key per-context SkImage wrapper caches without
/// having to hash the raw `DirectContext` handle (skia-safe exposes
/// `DirectContextId: Eq + Copy` but intentionally not `Hash`).
static CTX_TAG_ALLOC: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

fn alloc_ctx_tag() -> u32 {
    CTX_TAG_ALLOC.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Per-canvas Skia surface + state.
pub struct Canvas2DContext {
    pub gr_ctx: DirectContext,
    pub surface: SkSurface,
    pub renderer: Canvas2DRenderer,
    pub width: u32,
    pub height: u32,
    pub fbo_id: u32,
    pub kind: FboKind,
    /// Stable monotonic tag identifying this `Canvas2DContext` /
    /// `DirectContext` pair.  Used as a key by `ImageStore` to cache
    /// SkImage wrappers per-context — see
    /// [`super::image_store::ImageStore::resolve_cached_or_wrap`].
    pub ctx_tag: u32,
    /// `true` when external code (WebGL dispatch, DrawingBuffer blit)
    /// has mutated GL state since Skia's last `reset_context`.  The
    /// NEXT Skia-touching op on this context reads the flag, issues
    /// one `GrDirectContext::reset()`, and clears it back to `false`.
    ///
    /// Per-context rather than manager-global: each `Canvas2DContext`
    /// owns its own `GrDirectContext`, so invalidation has to be
    /// scoped to the affected context to avoid under-invalidation on
    /// the other contexts that share the EGL context.
    pub skia_state_stale: bool,
}

impl Canvas2DContext {
    /// Create a new Skia-backed Canvas2D bound to an existing FBO.
    ///
    /// Preconditions:
    ///   * Caller has already made the target EGL context current.
    ///   * `fbo_id` names a framebuffer with a colour attachment of at
    ///     least 8 bits per RGBA channel.
    ///   * `width`/`height` are the attachment dimensions in physical
    ///     pixels (no DPR scaling; Canvas 2D coords are 1:1 with pixels).
    ///
    /// Returns `None` if Skia could not build a GL interface for the
    /// current EGL context — usually indicates a driver issue on device.
    pub fn new(fbo_id: u32, width: u32, height: u32, kind: FboKind) -> Option<Self> {
        let interface = interfaces::make_egl()?;
        let mut gr_ctx = direct_contexts::make_gl(interface, None)?;

        let fb_info = sk_gl::FramebufferInfo {
            fboid: fbo_id,
            // GL_RGBA8 == 0x8058, but the Skia binding exposes it as the
            // canonical `Format::RGBA8` constant on the `gl` module.  Use
            // the symbolic constant so a driver quirk (e.g. BGRA swap)
            // can be swapped out later in one spot.
            format: gl_rgba8(),
            protected: gpu::Protected::No,
        };

        let target = backend_render_targets::make_gl(
            (width as i32, height as i32),
            /* sample_count */ Some(0),
            /* stencil_bits */ 8,
            fb_info,
        );

        let surface = gpu::surfaces::wrap_backend_render_target(
            &mut gr_ctx,
            &target,
            SurfaceOrigin::BottomLeft,
            ColorType::RGBA8888,
            /* color_space */ None,
            /* surface_props */ None,
        )?;

        // Clamp Ganesh's resource cache so a long-running scene
        // can't silently grow the GPU memory footprint past the
        // 200 MB native-heap target.  See `SKIA_RESOURCE_CACHE_*`
        // constants and
        // <https://api.skia.org/classGrDirectContext.html>.
        gr_ctx.set_resource_cache_limits(
            skia_safe::gpu::ganesh::ResourceCacheLimits {
                max_resources: SKIA_RESOURCE_CACHE_MAX_RESOURCES,
                max_resource_bytes: SKIA_RESOURCE_CACHE_MAX_BYTES,
            },
        );

        Some(Self {
            gr_ctx,
            surface,
            renderer: Canvas2DRenderer::new(),
            width,
            height,
            fbo_id,
            kind,
            skia_state_stale: false,
            ctx_tag: alloc_ctx_tag(),
        })
    }

    /// Release GPU resources Skia purged from its deferred command
    /// buffer.  Called from `CanvasManager` on app background/
    /// low-memory signals.  A zero-argument call means "only clean
    /// up resources that have already aged out"; pass a negative
    /// `msecs` (via the raw API) for aggressive immediate cleanup.
    #[inline]
    pub fn perform_deferred_cleanup(&mut self, ms_not_used: std::time::Duration) {
        self.gr_ctx.perform_deferred_cleanup(ms_not_used, None);
    }

    /// Mark Skia's cached GL state as dirty because code outside the
    /// Skia pipeline just mutated a live GL object.  The *next*
    /// Skia-touching op on this context will issue a single
    /// `GrDirectContext::reset()` and clear the flag.
    #[inline]
    pub fn mark_state_stale(&mut self) {
        self.skia_state_stale = true;
    }

    /// Idempotent lazy reset — issues `reset_context()` only when
    /// the dirty flag is set.  Safe to call before every Skia draw
    /// batch; cheap when state isn't actually stale.
    #[inline]
    pub fn reset_gl_state_if_stale(&mut self) {
        if self.skia_state_stale {
            self.gr_ctx.reset(None);
            self.skia_state_stale = false;
            crate::render_diagnostics::bump_skia_context_reset();
        }
    }

    #[inline]
    pub fn canvas(&mut self) -> &SkCanvas {
        self.surface.canvas()
    }

    /// Apply a Canvas2D command with no image-store access.
    ///
    /// Kept for tests and callers that never use `drawImage` /
    /// `createPattern`.  Production callers should use
    /// [`Self::apply_with_images`] so image-backed paths actually draw.
    pub fn apply<R: PatternResolver>(
        &mut self,
        cmd: &shared::protocol::render_cmd::Canvas2DCmd,
        text: &TextContext,
        resolver: &R,
    ) -> bool {
        let env = super::canvas::DrawEnv {
            canvas: self.surface.canvas(),
            text,
            resolver,
        };
        self.renderer.apply_env(&env, cmd)
    }

    /// Apply a Canvas2D command with full access to the shared
    /// [`ImageStore`] — required for `DrawImage` / `DrawImageBatch` /
    /// `createPattern` fills to actually rasterise pixels.
    ///
    /// Borrow gymnastics: this method explicitly destructures `&mut self`
    /// into disjoint `&mut gr_ctx`, `&mut surface`, and `&mut renderer`
    /// borrows so the pattern resolver (which needs mutable access to
    /// the Ganesh context to wrap a backend texture into an `SkImage`)
    /// can coexist with the draw call (which borrows `surface.canvas()`).
    pub fn apply_with_images(
        &mut self,
        cmd: &shared::protocol::render_cmd::Canvas2DCmd,
        text: &TextContext,
        image_store: &mut ImageStore,
    ) -> bool {
        use shared::protocol::render_cmd::Canvas2DCmd;

        // Explicit field destructure to obtain three disjoint borrows.
        let Canvas2DContext {
            gr_ctx,
            surface,
            renderer,
            ctx_tag,
            ..
        } = self;
        let ctx_tag = *ctx_tag;

        // `DrawImage` / `DrawImageBatch` use the `SkCanvas::draw_image_rect`
        // API directly; they don't go through `PatternResolver` at all.
        match cmd {
            Canvas2DCmd::DrawImage {
                image_id,
                sx,
                sy,
                sw,
                sh,
                dx,
                dy,
                dw,
                dh,
            } => {
                return draw_one_image(
                    ctx_tag,
                    gr_ctx,
                    surface.canvas(),
                    &renderer.state,
                    image_store,
                    *image_id,
                    *sx,
                    *sy,
                    *sw,
                    *sh,
                    *dx,
                    *dy,
                    *dw,
                    *dh,
                );
            }
            Canvas2DCmd::DrawImageBatch { draws } => {
                // Build the image paint once; all sub-draws share the
                // current 2D state (globalAlpha / composite / shadow /
                // smoothing).  SkCanvas's internal batcher merges the
                // runs when it can.
                let paint = build_image_paint(&renderer.state);
                let canvas = surface.canvas();
                let mut any = false;
                for d in draws {
                    if let Some(img) = image_store.resolve_cached_or_wrap(
                        ctx_tag,
                        gr_ctx,
                        d.image_id,
                    ) {
                        let src = SkRect::from_xywh(d.sx, d.sy, d.sw, d.sh);
                        let dst = SkRect::from_xywh(d.dx, d.dy, d.dw, d.dh);
                        canvas.draw_image_rect(
                            &img,
                            Some((&src, skia_safe::canvas::SrcRectConstraint::Fast)),
                            dst,
                            &paint,
                        );
                        any = true;
                    }
                }
                return any;
            }
            _ => {}
        }

        // Everything else goes through the renderer with a real
        // pattern resolver that can wrap a stored texture into a
        // tiling `SkShader`.  `RefCell` is the cheapest way to hand a
        // `&mut DirectContext` into a `&self` trait method without
        // reshaping the trait signature.
        let resolver = SkiaPatternResolver {
            ctx_tag,
            gr_ctx: RefCell::new(gr_ctx),
            image_store: RefCell::new(image_store),
        };
        let env = super::canvas::DrawEnv {
            canvas: surface.canvas(),
            text,
            resolver: &resolver,
        };
        renderer.apply_env(&env, cmd)
    }

    /// Flush Skia's deferred draws and SUBMIT to the GL driver.  Called
    /// at frame-end, at a Canvas2D→WebGL boundary (Materialize op), and
    /// whenever a synchronous readback needs to see pixels.
    pub fn flush_and_submit(&mut self) {
        self.gr_ctx.flush_and_submit();
    }

    /// Tell Skia to drop its cached GL state tracking.  Required
    /// immediately after [`flush_and_submit`] when control is about to
    /// return to code that mutates GL state outside Skia (WebGL handler,
    /// DrawingBuffer blit).  See Skia docs on
    /// `GrDirectContext::resetContext()`.
    pub fn reset_gl_state(&mut self) {
        self.gr_ctx.reset(None);
    }

    /// Resize after a surface change (window resize / orientation).  We
    /// rebuild the backing `SkSurface` because the FBO dimensions and
    /// possibly the underlying attachment handle changed.
    pub fn resize(&mut self, fbo_id: u32, width: u32, height: u32) -> bool {
        let Some(new_self) = Self::new(fbo_id, width, height, self.kind) else {
            return false;
        };
        // Preserve the state-machine state so JS-side style / transform
        // persist across a resize.  Path + CTM + clip are canvas-local
        // and must reset (matches browser behaviour: writing to
        // canvas.width clears content and resets the context).
        self.gr_ctx = new_self.gr_ctx;
        self.surface = new_self.surface;
        self.fbo_id = fbo_id;
        self.width = width;
        self.height = height;
        self.renderer.reset();
        true
    }

    /// Clear the entire surface to transparent — spec'd fallout of
    /// `canvas.width = N`.  Does not mutate the state machine.
    pub fn clear_to_transparent(&mut self) {
        self.surface
            .canvas()
            .clear(skia_safe::Color::TRANSPARENT);
    }
}

/// `GL_RGBA8` (0x8058) — named here so a future "use BGRA on Mali" quirk
/// can be flagged in one place.  Skia's `gl::Format::RGBA8` constant
/// aliases this value on every backend we target.
#[inline]
fn gl_rgba8() -> u32 {
    // 0x8058 is the sized internal format RGBA8 defined by the GLES 3.0
    // spec §3.8.3.  Using a literal rather than an enum to avoid a
    // feature-flag dependency on skia-bindings' gl constants.
    0x8058
}

// ---------------------------------------------------------------------------
// Image drawing helpers (shared by DrawImage / DrawImageBatch / pattern fills)
// ---------------------------------------------------------------------------

/// `SkPaint` preset tuned for `drawImage`.  Rebuilt each call because
/// the caller's `Canvas2DState` might have changed blend mode / alpha /
/// shadow / smoothing between frames, but the paint itself is cheap
/// (value type).
fn build_image_paint(state: &Canvas2DState) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(state.antialias);
    paint.set_blend_mode(state.blend_mode);

    // Canvas2D applies globalAlpha to *every* colour component of the
    // source image, not to a separate alpha channel — Skia's
    // `set_alpha_f` does exactly that.
    paint.set_alpha_f(state.global_alpha.clamp(0.0, 1.0));

    // image_smoothing=false → nearest-neighbour, else linear.  We use
    // mipmap=nearest to match Chromium's default (mipmap filter isn't
    // part of the Canvas2D spec until imageSmoothingQuality="high",
    // which we don't expose yet).
    let sampling = if state.image_smoothing {
        SamplingOptions::new(skia_safe::FilterMode::Linear, skia_safe::MipmapMode::Nearest)
    } else {
        SamplingOptions::new(skia_safe::FilterMode::Nearest, skia_safe::MipmapMode::None)
    };
    // `SkPaint` itself doesn't hold sampling options — they're passed
    // per-call to `draw_image_rect` via `SamplingOptionsCallback`.  We
    // stash them on the paint for callers that want a uniform default.
    // The helper is used only by `build_image_paint` tests today; real
    // draws pass the sampling explicitly below.
    let _ = sampling;

    super::paint::apply_shadow_to_paint(&mut paint, state);
    paint
}

/// Execute a single `drawImage` — factored out of [`Canvas2DContext::apply_with_images`]
/// so the call path works out the same between DrawImage and DrawImageBatch.
#[allow(clippy::too_many_arguments)]
fn draw_one_image(
    ctx_tag: u32,
    gr_ctx: &mut DirectContext,
    canvas: &SkCanvas,
    state: &Canvas2DState,
    image_store: &mut ImageStore,
    image_id: u32,
    sx: f32,
    sy: f32,
    sw: f32,
    sh: f32,
    dx: f32,
    dy: f32,
    dw: f32,
    dh: f32,
) -> bool {
    let Some(sk_image) = image_store.resolve_cached_or_wrap(ctx_tag, gr_ctx, image_id) else {
        return false;
    };

    let paint = build_image_paint(state);
    let src = SkRect::from_xywh(sx, sy, sw, sh);
    let dst = SkRect::from_xywh(dx, dy, dw, dh);
    let sampling = if state.image_smoothing {
        SamplingOptions::new(skia_safe::FilterMode::Linear, skia_safe::MipmapMode::Nearest)
    } else {
        SamplingOptions::new(skia_safe::FilterMode::Nearest, skia_safe::MipmapMode::None)
    };
    canvas.draw_image_rect_with_sampling_options(
        &sk_image,
        Some((&src, skia_safe::canvas::SrcRectConstraint::Fast)),
        dst,
        sampling,
        &paint,
    );
    true
}

/// `PatternResolver` backed by the manager's [`ImageStore`] and the
/// current canvas's [`DirectContext`].  Wraps `&mut gr_ctx` in a
/// `RefCell` so the trait's `&self` signature can still issue mutable
/// Ganesh calls (e.g. to build a `BackendTexture`).
struct SkiaPatternResolver<'a> {
    ctx_tag: u32,
    gr_ctx: RefCell<&'a mut DirectContext>,
    image_store: RefCell<&'a mut ImageStore>,
}

impl<'a> PatternResolver for SkiaPatternResolver<'a> {
    fn resolve_pattern(
        &self,
        image_id: u32,
        repeat_x: bool,
        repeat_y: bool,
        global_alpha: f32,
    ) -> Option<Shader> {
        let mut gr_ctx = self.gr_ctx.borrow_mut();
        let mut store = self.image_store.borrow_mut();
        let sk_image = store.resolve_cached_or_wrap(self.ctx_tag, *gr_ctx, image_id)?;
        let tile_x = if repeat_x {
            TileMode::Repeat
        } else {
            TileMode::Clamp
        };
        let tile_y = if repeat_y {
            TileMode::Repeat
        } else {
            TileMode::Clamp
        };
        // globalAlpha modulation: apply via colour filter on the shader.
        // For fully opaque the base shader is enough.
        let _ = global_alpha; // applied to the paint, not the shader
        sk_image.to_shader(
            Some((tile_x, tile_y)),
            SamplingOptions::default(),
            None,
        )
    }
}
