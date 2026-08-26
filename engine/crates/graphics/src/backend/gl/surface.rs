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
    Canvas as SkCanvas, ColorType, Matrix, Paint, Rect as SkRect, SamplingOptions, Shader,
    Surface as SkSurface, TileMode,
    gpu::{
        self, DirectContext, SurfaceOrigin, backend_render_targets, direct_contexts, gl as sk_gl,
    },
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
/// which is catastrophic when we have multiple Canvas2DContexts on
/// an Android device with 2 GB of system RAM.
///
/// We cap each context's Ganesh cache so the aggregate across all
/// live contexts stays within the 200 MB native-heap target.  Two
/// constants carve up that budget:
///
/// * `SKIA_RESOURCE_CACHE_BUDGET_BYTES` is the *aggregate* ceiling
///   we're willing to hand Skia across all live canvases.
/// * Each context's per-instance cap is
///   `max(MIN_PER_CTX_BYTES, budget / live_ctxs)` -- i.e. a single
///   onscreen canvas still gets the full 32 MiB, two canvases get
///   16 MiB each, four get 8 MiB each, and so on.  The minimum
///   keeps a tiny-but-active offscreen canvas above the glyph-atlas
///   working set.
///
/// Call [`Canvas2DContext::rebalance_resource_cache`] after any
/// canvas create / destroy so existing contexts pick up the new
/// share.  See Skia `GrDirectContext::setResourceCacheLimits`:
/// <https://api.skia.org/classGrDirectContext.html>.
/// Default aggregate Skia resource cache budget (32 MiB).  Kept
/// as the process-wide lower bound; individual tiers may raise
/// it through [`set_skia_resource_cache_budget`].
const DEFAULT_SKIA_RESOURCE_CACHE_BUDGET_BYTES: usize = 32 * 1024 * 1024;
/// Minimum per-context cap.  See [`per_ctx_resource_cache_bytes`]
/// — used so a tiny offscreen canvas still gets enough room for
/// its glyph atlas working set.
const MIN_PER_CTX_BYTES: usize = 4 * 1024 * 1024;
const SKIA_RESOURCE_CACHE_MAX_RESOURCES: usize = 1 << 14;

/// Runtime-tunable budget.  Set at engine init based on
/// `DeviceCapabilities::tier()` and lowered on
/// `onTrimMemory` hooks (P1-12).  The atomic is `AtomicUsize`
/// because reads happen on every canvas create / destroy and
/// we want the load to be lock-free.
static SKIA_RESOURCE_CACHE_BUDGET_BYTES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(DEFAULT_SKIA_RESOURCE_CACHE_BUDGET_BYTES);

/// Tune the aggregate resource-cache budget at runtime.
///
/// One call site: engine init, once the device tier has been detected, as
/// `set_skia_resource_cache_budget(tier_budget(tier))`. A low-memory signal
/// deliberately does *not* come through here — see [`low_memory_per_ctx_bytes`].
///
/// Existing contexts pick up the new cap the next time
/// [`Canvas2DContext::rebalance_resource_cache`] runs (driven by
/// canvas create / destroy).  To force an immediate rebalance,
/// the manager can call `rebalance_resource_cache` itself for
/// every live context.
pub fn set_skia_resource_cache_budget(bytes: usize) {
    SKIA_RESOURCE_CACHE_BUDGET_BYTES.store(
        bytes.max(MIN_PER_CTX_BYTES),
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// Suggested per-tier budget.  Numbers match Chromium-mobile
/// defaults on equivalent hardware and have been validated
/// against the 200 MB native-heap target on TierA devices.
pub fn tier_budget(tier: crate::device_caps::DeviceTier) -> usize {
    use crate::device_caps::DeviceTier;
    match tier {
        DeviceTier::TierA => 96 * 1024 * 1024,
        DeviceTier::TierB => 48 * 1024 * 1024,
    }
}

/// Aggregate ceiling a low-memory signal squeezes Skia down to.  Dropping below
/// `MIN_PER_CTX_BYTES` per context wouldn't be useful — Skia would re-evict on the
/// very next draw.
const LOW_MEMORY_AGGREGATE_BYTES: usize = 16 * 1024 * 1024;

/// Compute the per-context byte cap for the number of live
/// `Canvas2DContext`s **in the process**.
///
/// The count has to be process-wide because the budget is. It used to be passed in
/// as one `CanvasManager`'s own `contexts_2d.len()`, and there is one manager per
/// Session — so two Sessions each holding one canvas each divided the whole
/// aggregate budget by one, and the process handed Skia twice the ceiling this
/// module says it is willing to hand out. N Sessions meant N times.
///
/// Convergence is lazy, and that is forced rather than chosen: a Skia
/// `DirectContext` may only be touched from the render thread that owns it, so a
/// Session cannot rebalance another Session's contexts. A newly created context
/// takes the smaller share at once; already-live contexts in other Sessions keep
/// their larger cap until their own next canvas create or destroy. The overshoot is
/// bounded by what those contexts had already been granted.
#[inline]
pub(crate) fn per_ctx_resource_cache_bytes() -> usize {
    per_ctx_share(SKIA_RESOURCE_CACHE_BUDGET_BYTES.load(std::sync::atomic::Ordering::Relaxed))
}

/// The per-context cap a low-memory signal squeezes to.
///
/// **Not a budget, and that is the whole point.** A memory warning arrives per
/// Session — the host relays one Android `onTrimMemory` once for each — and it used
/// to be answered by *storing* 16 MiB as the process budget, which only engine init
/// ever raised again. So one game's warning capped every other game's canvases for
/// the life of the process. What a warning actually needs is a release, and Skia
/// releases when a lower cap is installed, so this figure is handed to
/// [`Canvas2DContext::trim_resource_cache`] and the ordinary share is restored in the
/// same call. Nothing outside that call is left capped, and a second Session relaying
/// the same signal finds nothing left to free rather than compounding the first.
#[inline]
pub(crate) fn low_memory_per_ctx_bytes() -> usize {
    per_ctx_share(LOW_MEMORY_AGGREGATE_BYTES)
}

/// One aggregate ceiling's share, for the contexts the process actually has.
///
/// Private, and takes no count: a caller that could pass the divisor is how the
/// numerator and the denominator came to have different scopes.
#[inline]
fn per_ctx_share(aggregate: usize) -> usize {
    let n = LIVE_CANVAS_CONTEXTS
        .load(std::sync::atomic::Ordering::Relaxed)
        .max(1);
    (aggregate / n).max(MIN_PER_CTX_BYTES)
}

/// Live `Canvas2DContext` count, across every Session in the process.
static LIVE_CANVAS_CONTEXTS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Membership in [`LIVE_CANVAS_CONTEXTS`], tied to a context's lifetime.
///
/// A field of `Canvas2DContext` rather than a pair of manual calls: the compiler
/// then refuses to build a context that is not counted, and the decrement cannot be
/// forgotten on any exit path — including a construction that fails after the
/// counter was raised.
pub(crate) struct LiveContextCount;

impl LiveContextCount {
    fn enrol() -> Self {
        LIVE_CANVAS_CONTEXTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self
    }
}

impl Drop for LiveContextCount {
    fn drop(&mut self) {
        LIVE_CANVAS_CONTEXTS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Monotonic allocator for `Canvas2DContext` identity tags.  Used by
/// [`ImageStore`] to key per-context SkImage wrapper caches without
/// having to hash the raw `DirectContext` handle (skia-safe exposes
/// `DirectContextId: Eq + Copy` but intentionally not `Hash`).
static CTX_TAG_ALLOC: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

fn alloc_ctx_tag() -> u32 {
    CTX_TAG_ALLOC.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Thin newtype around `skia_safe::gpu::DirectContext` (P2-3).
///
/// Exists to give the rest of the renderer a bounded API surface
/// that documents *which* Ganesh operations we actually use —
/// callers that want to reach past this wrapper still can via
/// [`Self::inner_mut`], but the presence of this type lets a
/// reviewer audit new Skia entry points by grepping for
/// `CanvasGr::` rather than trawling every file that imports
/// `skia_safe::gpu::DirectContext`.
///
/// The current engine already stores `gr_ctx: DirectContext`
/// directly on `Canvas2DContext` for backwards-compat with the
/// existing call graph; migrating each call site to use
/// `CanvasGr` is a follow-up (see `AUDIT.md` P2-3).  Until then
/// the newtype is exposed here as a documentation anchor and a
/// place to hang a narrow API once the migration starts.
#[allow(dead_code)]
pub(crate) struct CanvasGr {
    inner: DirectContext,
}

#[allow(dead_code)]
impl CanvasGr {
    /// Wrap an existing `DirectContext`.
    #[inline]
    pub(crate) fn new(inner: DirectContext) -> Self {
        Self { inner }
    }

    /// Access the raw Ganesh handle.  Kept `pub(crate)` so the
    /// boundary isn't accidentally widened to downstream crates;
    /// internal callers migrating to [`CanvasGr`] can use this
    /// during the transition without rewriting the full call
    /// stack.
    #[inline]
    pub(crate) fn inner_mut(&mut self) -> &mut DirectContext {
        &mut self.inner
    }

    /// Narrow `reset` wrapper: accepts a `skia_safe`-agnostic
    /// bitmask and passes it through.  Encourages call sites to
    /// use the named constants from `gr_state_bits` instead of
    /// raw Skia types.
    #[inline]
    pub(crate) fn reset(&mut self, bits: Option<u32>) {
        self.inner.reset(bits);
    }

    #[inline]
    pub(crate) fn flush_and_submit(&mut self) {
        self.inner.flush_and_submit();
    }

    #[inline]
    pub(crate) fn perform_deferred_cleanup(&mut self, not_used: std::time::Duration) {
        self.inner.perform_deferred_cleanup(not_used, None);
    }
}

/// Outcome of [`Canvas2DContext::try_fast_path_draw_image`].
///
/// Making the fast-path result explicit (P2-12) prevents the
/// previous "match-and-fall-through" pattern from silently
/// dropping new `DrawImage*` variants into the slow path
/// unannounced — a new variant that a contributor forgets to add
/// to the fast-path match now either produces the correct
/// fallback (handled cleanly) or trips a compiler warning if
/// someone adds `Handled` without a body.  The enum is private
/// to the backend because it's an implementation detail of the
/// dispatch layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FastPathOutcome {
    /// Command was handled inside the fast path.  The inner
    /// boolean mirrors the rest of the backend API's "was this
    /// a real render" convention: `true` means pixels were
    /// submitted, `false` means the command resolved to a
    /// no-op (e.g. a `DrawImageBatch` with zero resolvable
    /// entries).
    Handled(bool),
    /// Command was not a fast-path candidate.  The caller must
    /// route it through the generic resolver-based dispatch.
    Fallback,
}

/// Bit flags mirroring Skia's `GrGLBackendState` (see
/// `skia/include/gpu/ganesh/GrTypes.h`).  Only the bits Migo
/// actually needs are named; the rest fall through to
/// `skia_bindings::kAll_GrBackendState` via [`GrStateBits::ALL`].
///
/// Using a bitmask instead of the previous boolean lets the
/// per-context staleness tracker invalidate *exactly* the GL state
/// that external code mutated — e.g. an AHB import only dirties
/// the active texture binding, so there is no reason to force
/// Skia to re-send viewport / blend / program / stencil /
/// pixel-store state on the next draw.  Matches the idiomatic
/// Skia integration pattern documented in `GrDirectContext.h`.
pub mod gr_state_bits {
    /// Render target / framebuffer binding.
    pub const RENDER_TARGET: u32 = 1 << 0;
    /// Active texture unit + bound texture (includes sampler
    /// objects for ES 3.0+).  Set by AHB import and by raw GL
    /// texture creation that bypasses Skia.
    pub const TEXTURE_BINDING: u32 = 1 << 1;
    /// Scissor + viewport.
    pub const VIEW: u32 = 1 << 2;
    /// Blend enable / equation / factors.
    pub const BLEND: u32 = 1 << 3;
    /// Pixel-store `glPixelStorei` (UNPACK_ALIGNMENT etc).
    pub const PIXEL_STORE: u32 = 1 << 7;
    /// Currently bound program + active uniforms / attrib bindings.
    pub const PROGRAM: u32 = 1 << 8;

    /// "All known" mask.  Prefer this over `u32::MAX` so the
    /// invalidation stays inside Skia's declared enum range — on
    /// drivers that add new bits in the future we still want
    /// `reset_context(ALL)` to be equivalent to passing `None`.
    pub const ALL: u32 = 0xffff;
}

/// Per-canvas Skia surface + state.
pub struct Canvas2DContext {
    /// Keeps this context in the process-wide live count for as long as it exists,
    /// which is what makes the Skia budget's denominator match its numerator.
    _counted: LiveContextCount,
    pub gr_ctx: DirectContext,
    pub surface: SkSurface,
    /// The assembled GL interface this context's `GrDirectContext` was built
    /// from, kept so [`Canvas2DContext::resize`] can rebuild the context
    /// without needing the entry-point loader threaded back in. It is
    /// reference-counted on the Skia side, so holding it costs a refcount.
    interface: sk_gl::Interface,
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
    /// Bitmask of [`gr_state_bits`] describing which slices of GL
    /// state external code (WebGL dispatch, DrawingBuffer blit, AHB
    /// import) has mutated since Skia's last `reset_context`.
    ///
    /// The NEXT Skia-touching op on this context reads the mask,
    /// issues one `GrDirectContext::reset(Some(mask))`, and clears
    /// the mask back to 0.  `0` means "Skia's tracked state is in
    /// sync"; non-zero means "pass the mask to Skia".
    ///
    /// Per-context rather than manager-global: each `Canvas2DContext`
    /// owns its own `GrDirectContext`, so invalidation has to be
    /// scoped to the affected context to avoid under-invalidation on
    /// the other contexts that share the EGL context.
    pub skia_state_stale: u32,
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
    ///
    /// `load_gl` resolves a GL entry point. It must come from the *same* EGL
    /// implementation the caller injected for this manager, which is why it is
    /// a parameter rather than something Skia looks up for itself: Skia's own
    /// `interfaces::make_egl()` calls whichever `eglGetProcAddress` Skia was
    /// linked against, and on a host that supplies its own EGL (the Linux X11
    /// and Wayland presenters, or a test double) that is not necessarily the
    /// one Migo is driving. Assembling the interface from the injected loader
    /// makes both halves of the process agree by construction.
    ///
    /// It also removes Skia's need to link EGL at all, which is what lets this
    /// crate link on Windows: Skia's GL-interface selection is an if/else-if
    /// chain where `skia_use_egl` makes the Windows branch unreachable, while
    /// skia-bindings emits its `GrGLInterfaces::MakeWin` wrapper on every
    /// Windows build regardless — an unresolvable pair for as long as we asked
    /// Skia for the EGL interface.
    pub fn new(
        fbo_id: u32,
        width: u32,
        height: u32,
        kind: FboKind,
        load_gl: &dyn Fn(&str) -> *const std::ffi::c_void,
    ) -> Option<Self> {
        let interface = sk_gl::Interface::new_load_with(|symbol| load_gl(symbol))?;
        Self::with_interface(interface, fbo_id, width, height, kind)
    }

    /// Build a context around an already-assembled GL interface.
    ///
    /// Split out so `resize` can rebuild the `GrDirectContext` from the
    /// interface this context already holds. Re-assembling one there would mean
    /// carrying the loader through every resize call site, across borrows of
    /// the canvas manager that the entry-point table is a sibling field of.
    fn with_interface(
        interface: sk_gl::Interface,
        fbo_id: u32,
        width: u32,
        height: u32,
        kind: FboKind,
    ) -> Option<Self> {
        let mut gr_ctx = direct_contexts::make_gl(interface.clone(), None)?;

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
        // 200 MB native-heap target.  Start at the single-context
        // budget; `rebalance_resource_cache` shrinks the per-cap
        // as additional canvases come online.  See
        // `per_ctx_resource_cache_bytes` and
        // <https://api.skia.org/classGrDirectContext.html>.
        // Enrolled before the cap is computed so this context is in its own divisor.
        let counted = LiveContextCount::enrol();
        gr_ctx.set_resource_cache_limits(skia_safe::gpu::ganesh::ResourceCacheLimits {
            max_resources: SKIA_RESOURCE_CACHE_MAX_RESOURCES,
            max_resource_bytes: per_ctx_resource_cache_bytes(),
        });

        Some(Self {
            _counted: counted,
            gr_ctx,
            surface,
            interface,
            renderer: Canvas2DRenderer::new(),
            width,
            height,
            fbo_id,
            kind,
            skia_state_stale: 0,
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

    /// Mark every slice of Skia's cached GL state dirty because
    /// external code mutated multiple categories at once — e.g. a
    /// full WebGL batch that changed program, textures, viewport,
    /// and blend state.  Equivalent to passing `None` to
    /// `DirectContext::reset`.
    ///
    /// Prefer [`Self::mark_state_stale_bits`] when the caller
    /// knows exactly which slice changed (AHB import only
    /// mutates `TEXTURE_BINDING`, a DrawingBuffer blit only
    /// mutates `RENDER_TARGET`): the narrower invalidation lets
    /// Skia skip re-sending the rest of its tracked state on the
    /// next draw, saving ~a dozen GL calls per Canvas2D↔WebGL
    /// boundary.
    #[inline]
    pub fn mark_state_stale(&mut self) {
        self.skia_state_stale = gr_state_bits::ALL;
    }

    /// Mark a specific subset of Skia's cached GL state dirty.
    /// The bits OR together with any previously recorded
    /// staleness — a subsequent draw will see the union.
    #[inline]
    pub fn mark_state_stale_bits(&mut self, bits: u32) {
        self.skia_state_stale |= bits;
    }

    /// Re-apply the resource-cache byte cap for the current number
    /// of live `Canvas2DContext`s.  Called by the manager after any
    /// canvas create / destroy so the aggregate Skia cache budget
    /// stays pinned at
    /// [`SKIA_RESOURCE_CACHE_BUDGET_BYTES`] regardless of how
    /// many contexts are live.
    #[inline]
    pub fn rebalance_resource_cache(&mut self) {
        self.gr_ctx
            .set_resource_cache_limits(skia_safe::gpu::ganesh::ResourceCacheLimits {
                max_resources: SKIA_RESOURCE_CACHE_MAX_RESOURCES,
                max_resource_bytes: per_ctx_resource_cache_bytes(),
            });
    }

    /// Squeeze this context to the low-memory share, then restore the share the
    /// aggregate budget allows.
    ///
    /// Installing a lower cap is what makes Skia purge: `setResourceCacheLimits`
    /// evicts down to the new figure before returning, so the release lands inside
    /// this call and the cap does not have to stay behind to have had its effect.
    /// That is the difference from what this replaced — a *stored* low budget, which
    /// no signal ever lifted and which therefore capped every Session in the process,
    /// not just the one that was asked to trim.
    #[inline]
    pub fn trim_resource_cache(&mut self) {
        self.gr_ctx
            .set_resource_cache_limits(skia_safe::gpu::ganesh::ResourceCacheLimits {
                max_resources: SKIA_RESOURCE_CACHE_MAX_RESOURCES,
                max_resource_bytes: low_memory_per_ctx_bytes(),
            });
        self.rebalance_resource_cache();
    }

    /// Idempotent lazy reset — issues `reset_context(bits)` only
    /// when the dirty mask is non-zero.  Safe to call before every
    /// Skia draw batch; cheap when state isn't actually stale.
    ///
    /// If the mask saturates to [`gr_state_bits::ALL`] we still
    /// pass `None` (i.e. `kAll_GrBackendState`) to Skia so any
    /// hypothetical future GL backend state bit outside our
    /// enumeration is also invalidated; partial masks get mapped
    /// 1:1 to `Some(bits)`.
    #[inline]
    pub fn reset_gl_state_if_stale(&mut self) {
        if self.skia_state_stale == 0 {
            return;
        }
        let reset_arg = if self.skia_state_stale == gr_state_bits::ALL {
            None
        } else {
            Some(self.skia_state_stale)
        };
        self.gr_ctx.reset(reset_arg);
        self.skia_state_stale = 0;
        crate::render_diagnostics::bump_skia_context_reset();
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
            text: Some(text),
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
    ///
    /// The dispatch logic is split into two steps so future readers
    /// don't have to infer which commands take the fast path.  See
    /// [`FastPathOutcome`] and [`Self::try_fast_path_draw_image`].
    pub fn apply_with_images(
        &mut self,
        cmd: &shared::protocol::render_cmd::Canvas2DCmd,
        text: Option<&TextContext>,
        image_store: &mut ImageStore,
    ) -> bool {
        // P2-12: dispatch via an explicit `FastPathOutcome` rather
        // than a loosely-documented `match ... _ => {}` + fall-through.
        match self.try_fast_path_draw_image(cmd, image_store) {
            FastPathOutcome::Handled(painted) => return painted,
            FastPathOutcome::Fallback => {}
        }

        // Everything else goes through the renderer with a real
        // pattern resolver that can wrap a stored texture into a
        // tiling `SkShader`.  `RefCell` is the cheapest way to hand a
        // `&mut DirectContext` into a `&self` trait method without
        // reshaping the trait signature.
        let Canvas2DContext {
            gr_ctx,
            surface,
            renderer,
            ctx_tag,
            ..
        } = self;
        let ctx_tag = *ctx_tag;
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

    /// Draw-image fast path: `DrawImage` / `DrawImageBatch` talk to
    /// `SkCanvas::draw_image_rect` directly without building a
    /// `SkiaPatternResolver`, because the pattern resolver isn't
    /// needed for the common sprite blit case and its
    /// `RefCell<&mut DirectContext>` construction isn't free.
    ///
    /// Returns [`FastPathOutcome::Handled(painted)`] when the command
    /// matched a fast path (with `painted = true` iff the draw
    /// actually emitted pixels), or [`FastPathOutcome::Fallback`]
    /// when the caller should route the command through the
    /// generic `apply_env` path.  Any new `Canvas2DCmd` variant that
    /// wants a fast path **must** add a branch here; forgetting to
    /// do so only costs the extra resolver construction, never
    /// correctness.
    fn try_fast_path_draw_image(
        &mut self,
        cmd: &shared::protocol::render_cmd::Canvas2DCmd,
        image_store: &mut ImageStore,
    ) -> FastPathOutcome {
        use shared::protocol::render_cmd::Canvas2DCmd;

        let Canvas2DContext {
            gr_ctx,
            surface,
            renderer,
            ctx_tag,
            ..
        } = self;
        let ctx_tag = *ctx_tag;

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
                let painted = draw_one_image(
                    ctx_tag,
                    gr_ctx,
                    surface.canvas(),
                    renderer,
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
                FastPathOutcome::Handled(painted)
            }
            Canvas2DCmd::DrawImageBatch { draws } => {
                // Build the image paint once; all sub-draws share the
                // current 2D state (globalAlpha / composite / shadow /
                // smoothing).  Snapshot the state for the build
                // closure so the `&mut renderer` borrow needed by
                // `acquire_image_paint` doesn't conflict with the
                // `&renderer.state` the closure would otherwise
                // capture.  `Canvas2DState` is `Clone`; the copy is
                // cheap relative to the Paint construction we're
                // trying to skip in the cache-hit case.
                let state_snapshot = renderer.state.clone();
                let paint = renderer.acquire_image_paint(|| build_image_paint(&state_snapshot));
                let canvas = surface.canvas();
                let mut any = false;
                for d in draws {
                    if let Some(img) =
                        image_store.resolve_cached_or_wrap(ctx_tag, gr_ctx, d.image_id)
                    {
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
                FastPathOutcome::Handled(any)
            }
            _ => FastPathOutcome::Fallback,
        }
    }

    /// Flush Skia's deferred draws and SUBMIT to the GL driver.  Called
    /// at frame-end, at a Canvas2D→WebGL boundary (Materialize op), and
    /// whenever a synchronous readback needs to see pixels.
    pub fn flush_and_submit(&mut self) {
        self.gr_ctx.flush_and_submit();
    }

    /// Read back a Canvas2D sub-rectangle as unpremultiplied RGBA8888.
    ///
    /// This is the renderer-side implementation of Canvas2D
    /// `getImageData()`, so it intentionally matches the JS-visible
    /// `ImageData` layout rather than WebGL's `readPixels` contract.
    pub fn read_image_data(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> shared::error::EngineResult<Vec<u8>> {
        self.reset_gl_state_if_stale();
        self.flush_and_submit();
        read_surface_rgba_unpremul(&mut self.surface, x, y, width, height).ok_or_else(|| {
            shared::error::EngineError::new(shared::error::ErrorCode::Internal)
                .with_msg("Canvas2D getImageData read_pixels failed")
        })
    }

    /// Tell Skia to drop its cached GL state tracking.  Required
    /// immediately after [`flush_and_submit`] when control is about to
    /// return to code that mutates GL state outside Skia (WebGL handler,
    /// DrawingBuffer blit).  See Skia docs on
    /// `GrDirectContext::resetContext()`.
    pub fn reset_gl_state(&mut self) {
        self.gr_ctx.reset(None);
    }

    /// Mark the underlying Ganesh context abandoned so dropping this
    /// wrapper will not issue GL object destruction against the wrong
    /// or already-dead EGL context.
    pub fn abandon(&mut self) {
        self.gr_ctx.abandon();
    }

    /// Resize after a surface change (window resize / orientation).  We
    /// rebuild the backing `SkSurface` because the FBO dimensions and
    /// possibly the underlying attachment handle changed.
    ///
    /// `image_store` is a hole for purging the SkImage wrapper cache
    /// this context previously populated.  The wrappers are bound to
    /// the *old* `GrDirectContext`; once we swap in a new one they're
    /// stale — continuing to hand them out would produce undefined
    /// rendering on the next `drawImage` / pattern lookup.  To avoid
    /// the GrContext-identity trap we also allocate a fresh `ctx_tag`,
    /// so any wrapper entry the purge missed (e.g. re-entrant callers)
    /// becomes unreachable via the new cache key.
    pub fn resize(
        &mut self,
        fbo_id: u32,
        width: u32,
        height: u32,
        image_store: &mut ImageStore,
    ) -> bool {
        let Some(new_self) =
            Self::with_interface(self.interface.clone(), fbo_id, width, height, self.kind)
        else {
            return false;
        };
        // Drop every SkImage wrapper this context produced: they hold
        // a `GrDirectContext` pointer that's about to be dropped, and
        // reusing them post-swap is undefined behaviour inside Skia.
        image_store.purge_wrappers_for_context(self.ctx_tag);
        // `renderer.reset()` below returns the whole state machine to spec
        // defaults, which is what `canvas.width = N` is defined to do and what
        // the JS setters assume when they reset their shadow to match.
        //
        // A caller resizing for any OTHER reason -- the platform handed us a
        // different surface -- has to carry the state across itself, because
        // the content did not ask for a reset and will never re-send a value it
        // believes is still in force. `resize_canvas_for_surface_change` is
        // that caller.
        self.gr_ctx = new_self.gr_ctx;
        self.surface = new_self.surface;
        self.fbo_id = fbo_id;
        self.width = width;
        self.height = height;
        // Fresh ctx_tag: guarantees the next `resolve_cached_or_wrap`
        // call cannot collide with any residual wrapper from the
        // previous GrDirectContext.
        self.ctx_tag = new_self.ctx_tag;
        self.renderer.reset();
        true
    }

    /// The drawing-state machine (styles, alpha, line and text settings) this
    /// context holds.
    ///
    /// JS keeps a shadow copy of every one of these setters and skips sending a
    /// value it believes is already current, which makes this the authoritative
    /// half of a pair split across the ABI. Any path that replaces a context
    /// must carry the state across: a replacement that comes up at spec
    /// defaults desynchronises the two halves permanently, because the content
    /// has no way to learn its state was discarded and will never re-send it.
    /// `resize` preserves this by keeping `renderer`; a destroy-and-recreate
    /// has to ask for it explicitly -- and so does `resize`, which resets it.
    pub fn drawing_state(&self) -> Canvas2DState {
        self.renderer.state.clone()
    }

    /// Adopt drawing state captured from the context this one replaces.
    ///
    /// Re-applies the transform as well, because `Canvas2DState::ctm` is a
    /// mirror of `SkCanvas`'s own CTM rather than the thing Skia draws with,
    /// and the `SkCanvas` behind a replacement surface starts at identity.
    /// Adopting the mirror alone would leave the two disagreeing -- the damage
    /// classifier transforms rectangles with the mirror -- and would drop a
    /// transform the content set once and, like every other de-duplicated
    /// setter, will never send again.
    pub fn adopt_drawing_state(&mut self, state: Canvas2DState) {
        let [a, b, c, d, e, f] = state.ctm;
        self.renderer.state = state;
        let m = Matrix::new_all(a, c, e, b, d, f, 0.0, 0.0, 1.0);
        self.surface.canvas().set_matrix(&skia_safe::M44::from(m));
    }

    /// Clear the entire surface to transparent — spec'd fallout of
    /// `canvas.width = N`.  Does not mutate the state machine.
    pub fn clear_to_transparent(&mut self) {
        self.surface.canvas().clear(skia_safe::Color::TRANSPARENT);
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
        SamplingOptions::new(
            skia_safe::FilterMode::Linear,
            skia_safe::MipmapMode::Nearest,
        )
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

/// Execute a single `drawImage` - factored out of
/// [`Canvas2DContext::apply_with_images`] so the call path works
/// out the same between DrawImage and DrawImageBatch.
///
/// Routes the paint construction through
/// [`Canvas2DRenderer::acquire_image_paint`] so that a burst of
/// identical-style draws (the common UI case) only builds the
/// `SkPaint` once.
#[allow(clippy::too_many_arguments)]
fn draw_one_image(
    ctx_tag: u32,
    gr_ctx: &mut DirectContext,
    canvas: &SkCanvas,
    renderer: &mut super::canvas::Canvas2DRenderer,
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

    // Snapshot the state before crossing into `acquire_image_paint`
    // (which needs a `&mut renderer`).  `Canvas2DState` is `Clone`
    // and the copy is vastly cheaper than the Paint construction
    // the cache is there to skip.
    let state_snapshot = renderer.state.clone();
    let paint = renderer.acquire_image_paint(|| build_image_paint(&state_snapshot));
    let src = SkRect::from_xywh(sx, sy, sw, sh);
    let dst = SkRect::from_xywh(dx, dy, dw, dh);
    let sampling = if state_snapshot.image_smoothing {
        SamplingOptions::new(
            skia_safe::FilterMode::Linear,
            skia_safe::MipmapMode::Nearest,
        )
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
        sk_image.to_shader(Some((tile_x, tile_y)), SamplingOptions::default(), None)
    }
}

pub(crate) fn read_surface_rgba_unpremul(
    surface: &mut SkSurface,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let byte_len = shared::protocol::render_cmd::checked_canvas_rgba_byte_len(width, height)?;
    let info = skia_safe::ImageInfo::new(
        skia_safe::ISize::new(width as i32, height as i32),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        None,
    );
    let row_bytes = usize::try_from(width).ok()?.checked_mul(4)?;
    let mut out = vec![0u8; byte_len];
    surface
        .read_pixels(&info, &mut out, row_bytes, (x, y))
        .then_some(out)
}

#[cfg(test)]
mod tests {
    use super::{
        LiveContextCount, MIN_PER_CTX_BYTES, SKIA_RESOURCE_CACHE_BUDGET_BYTES,
        low_memory_per_ctx_bytes, per_ctx_resource_cache_bytes, read_surface_rgba_unpremul,
        set_skia_resource_cache_budget,
    };
    use skia_safe::{AlphaType, Color, ColorType, ISize, ImageInfo, Paint, Rect, surfaces};
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;

    /// The budget and the live count are process-wide, so two tests moving them
    /// concurrently would read each other's values.
    static BUDGET_TESTS: Mutex<()> = Mutex::new(());

    #[test]
    fn read_surface_rgba_unpremul_returns_painted_pixels() {
        let info = ImageInfo::new(
            ISize::new(2, 2),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            None,
        );
        let mut surface = surfaces::raster(&info, None, None).expect("valid raster surface");
        surface.canvas().clear(Color::TRANSPARENT);

        let mut paint = Paint::default();
        paint.set_color(Color::from_argb(255, 10, 20, 30));
        surface
            .canvas()
            .draw_rect(Rect::from_xywh(0.0, 0.0, 1.0, 1.0), &paint);

        let pixels =
            read_surface_rgba_unpremul(&mut surface, 0, 0, 1, 1).expect("readback should work");

        assert_eq!(pixels, vec![10, 20, 30, 255]);
    }

    /// Section 6.5: the Skia budget's denominator has to span the process, because
    /// its numerator does.
    ///
    /// The guard is exercised directly rather than through `Canvas2DContext::new`,
    /// which needs an EGL context and a GPU. What that leaves uncovered is only
    /// whether a context is enrolled at all, and that is not a convention to test:
    /// `LiveContextCount` is a required field, so a context which is not counted does
    /// not compile.
    ///
    /// Serialised against the other budget test: both move process-wide state.
    #[test]
    fn the_per_context_cap_divides_the_budget_across_every_session_s_contexts() {
        let _serialised = BUDGET_TESTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let restore = SKIA_RESOURCE_CACHE_BUDGET_BYTES.load(Ordering::Relaxed);
        set_skia_resource_cache_budget(64 * 1024 * 1024);

        // No contexts anywhere: the divisor floors at one rather than dividing by zero.
        assert_eq!(per_ctx_resource_cache_bytes(), 64 * 1024 * 1024);

        // One context in this Session.
        let first = LiveContextCount::enrol();
        assert_eq!(per_ctx_resource_cache_bytes(), 64 * 1024 * 1024);

        // A second Session brings up its own. Before this counted process-wide, each
        // Session divided the whole budget by its own single context and the process
        // handed Skia 128 MiB against a 64 MiB ceiling.
        let second = LiveContextCount::enrol();
        assert_eq!(per_ctx_resource_cache_bytes(), 32 * 1024 * 1024);

        let third = LiveContextCount::enrol();
        let fourth = LiveContextCount::enrol();
        assert_eq!(per_ctx_resource_cache_bytes(), 16 * 1024 * 1024);

        // Deep enough that the per-context floor takes over from the share.
        let many: Vec<LiveContextCount> = (0..60).map(|_| LiveContextCount::enrol()).collect();
        assert_eq!(per_ctx_resource_cache_bytes(), MIN_PER_CTX_BYTES);
        drop(many);

        // The count is released by dropping, on every path, because it is a guard.
        drop((second, third, fourth));
        assert_eq!(per_ctx_resource_cache_bytes(), 64 * 1024 * 1024);
        drop(first);
        assert_eq!(per_ctx_resource_cache_bytes(), 64 * 1024 * 1024);

        set_skia_resource_cache_budget(restore);
    }

    #[test]
    fn the_budget_never_drops_below_one_context_s_floor() {
        let _serialised = BUDGET_TESTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let restore = SKIA_RESOURCE_CACHE_BUDGET_BYTES.load(Ordering::Relaxed);

        set_skia_resource_cache_budget(1);
        assert_eq!(per_ctx_resource_cache_bytes(), MIN_PER_CTX_BYTES);

        set_skia_resource_cache_budget(restore);
    }

    /// A memory warning must release bytes without capping anyone.
    ///
    /// The defect: one Session's `onTrimMemory` stored 16 MiB as *the* process
    /// budget, and only engine init ever raised it again — so one game's warning
    /// capped every other game's canvases for the life of the process. The squeeze is
    /// now a figure handed to Skia and immediately superseded, so what a Session's
    /// warning changes is what Skia holds, not what the process may hold.
    ///
    /// What this cannot see, stated rather than implied: whether
    /// `Canvas2DContext::trim_resource_cache` really installs the low cap before
    /// restoring the ordinary one, since a `DirectContext` needs an EGL context and a
    /// GPU. What it does cover is that no path can store the low figure — there is no
    /// longer a function that yields one.
    #[test]
    fn a_low_memory_squeeze_leaves_no_ceiling_behind() {
        let _serialised = BUDGET_TESTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let restore = SKIA_RESOURCE_CACHE_BUDGET_BYTES.load(Ordering::Relaxed);
        set_skia_resource_cache_budget(64 * 1024 * 1024);

        let ordinary = per_ctx_resource_cache_bytes();
        let squeezed = low_memory_per_ctx_bytes();
        assert!(
            squeezed < ordinary,
            "a squeeze that asks for {squeezed} of an allowed {ordinary} frees nothing"
        );
        assert_eq!(
            per_ctx_resource_cache_bytes(),
            ordinary,
            "asking for the squeeze changed what the process may hold, so it outlived \
             the call that asked for it"
        );

        // The same, with two Sessions' contexts live: both figures divide by the same
        // process-wide count, so the squeeze stays proportional rather than becoming
        // the floor as soon as a second game starts.
        let contexts = [LiveContextCount::enrol(), LiveContextCount::enrol()];
        assert_eq!(low_memory_per_ctx_bytes(), 8 * 1024 * 1024);
        assert_eq!(per_ctx_resource_cache_bytes(), 32 * 1024 * 1024);
        drop(contexts);

        set_skia_resource_cache_budget(restore);
    }
}
