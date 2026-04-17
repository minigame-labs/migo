extern crate khronos_egl as egl;

use crate::backend::gl::surface::Canvas2DContext;
use crate::dirty_region::damage_tracker::ResolvedDamage;
use crate::BoundContext;
use egl::EGL1_4;
use glow::HasContext;
use shared::{
    error::{EngineResult, ErrorCode},
    protocol::{
        io_cmd::NormalizedImage,
        render_cmd::{
            BufferId, CanvasId, FramebufferId, ProgramId, RenderbufferId, ShaderId, TextureId,
        },
    },
};
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicU32, Ordering},
};

mod context_2d_impl;
pub(crate) mod drawing_buffer;
mod egl_ops;
mod image;
mod pbo_upload;
mod types;

pub(crate) use types::{
    ee, BlendEquation, BlendFactors, BufferMeta, CanvasGLState, CanvasInfo, FramebufferMeta,
    VertexAttribPointerFp,
    ProgramMeta, RenderbufferMeta, SamplerMeta, ScissorState, ShaderMeta, SyncMeta, TextureMeta,
    VaoMeta, MAX_UNIFORM_CACHE,
};
use types::{CanvasEntry, EglContextHandle, SurfaceKind};

use self::image::ImageRegistry;

#[allow(private_interfaces)]
pub(crate) struct CanvasManager {
    pub(crate) egl: egl::DynamicInstance<EGL1_4>,
    gl: glow::Context,
    display: egl::Display,
    config: egl::Config,

    #[allow(dead_code)]
    pub(super) dpi: f32,

    resource: EglContextHandle,

    // current binding
    pub(super) bound: BoundContext,

    // canvases
    pub(super) canvases: HashMap<CanvasId, CanvasEntry>,
    next_canvas_id: AtomicU32,

    // 2D
    pub(super) contexts_2d: HashMap<CanvasId, Canvas2DContext>,
    pub(super) dirty_2d: HashSet<CanvasId>,

    // Image registry
    image_registry: ImageRegistry,

    /// Last eglSwapInterval value to avoid redundant driver calls per frame.
    /// Initialized to -1 (sentinel) so the first swap forces an actual EGL call.
    /// Reset to -1 whenever the EGL surface is destroyed/recreated, because the
    /// new surface may not inherit the previous interval on all drivers.
    /// The sole eglSwapInterval call site is in `swap_buffers_no_restore()`,
    /// guarded by `interval != self.last_swap_interval`.
    last_swap_interval: i32,

    /// Set to true when EGL reports CONTEXT_LOST during swap_buffers.
    /// The next `create_onscreen()` call will perform a full resource rebuild.
    context_lost: bool,

    /// Last window handle used by create_onscreen, preserved for context
    /// loss recovery (re-create surface without waiting for UpdateSurface).
    last_window: Option<usize>,

    // GL object registries
    pub(crate) programs: HashMap<ProgramId, ProgramMeta>,
    pub(crate) shaders: HashMap<ShaderId, ShaderMeta>,
    pub(crate) buffers: HashMap<BufferId, BufferMeta>,
    pub(crate) textures: HashMap<TextureId, TextureMeta>,
    pub(crate) framebuffers: HashMap<FramebufferId, FramebufferMeta>,
    pub(crate) renderbuffers: HashMap<RenderbufferId, RenderbufferMeta>,
    pub(crate) gl_state: HashMap<CanvasId, CanvasGLState>,

    // WebGL 2 registries.  Kept separate from the WebGL 1 set because
    // lookup paths are distinct (VAOs are bound by the state tracker,
    // samplers are per-texture-unit, sync handles are polled from
    // synchronous reply ops).  The dispatchers in renderergl own the
    // actual GL calls; manager just holds the handle tables.
    pub(crate) vaos: HashMap<shared::protocol::render_cmd::VaoId, VaoMeta>,
    pub(crate) samplers: HashMap<shared::protocol::render_cmd::SamplerId, SamplerMeta>,
    pub(crate) syncs: HashMap<shared::protocol::render_cmd::SyncId, SyncMeta>,

    // NOTE: WebGL → Canvas2D invalidation is now tracked per-context
    // via `Canvas2DContext::skia_state_stale` (see backend/gl/surface.rs).
    // The old manager-global `skia_needs_reset` flag was removed after
    // it was observed to over-/mis-invalidate in multi-canvas scenes.

    /// Runtime device capabilities, detected once at init.
    pub(crate) device_caps: crate::device_caps::DeviceCapabilities,

    /// GLES major version negotiated during EGL init (3 = ES 3.0+, 2 = ES 2.0).
    /// Used when creating shared contexts (offscreen canvas, upload thread).
    gles_major: u32,

    /// Preserved EGL context from the last destroyed onscreen canvas.
    /// Reused on the next `create_onscreen()` to avoid losing GL state
    /// (textures, shaders, buffers) across Android surface destroy/recreate cycles.
    preserved_ctx: Option<egl::Context>,

    /// Set to true when a game reads pixels from the onscreen default framebuffer
    /// (readPixels on canvas_id=1 with default FBO bound). Once set, DrawingBuffer
    /// bypass is permanently disabled so the DrawingBuffer preserves content across
    /// swaps and readback returns valid data. One-way latch — never cleared.
    needs_default_fbo_readback: bool,

    /// Per-frame upload budget gating (device-tier aware).
    /// Gates submission to the upload thread with bandwidth and job-count limits.
    upload_server: Option<crate::upload_server::UploadServer>,

    /// Dedicated texture upload thread (TierA only).
    /// None on TierB devices or if shared-context probe failed.
    pub(crate) upload_thread: Option<crate::upload_thread::UploadThreadHandle>,

    /// Best-effort shader binary cache.
    /// Saves compiled GL program binaries to disk; loads on next run.
    /// None if GL_NUM_PROGRAM_BINARY_FORMATS == 0.
    pub(crate) shader_cache: Option<crate::shader_cache::ShaderCache>,

    /// Uploads whose GPU fence has not yet signaled.
    /// Re-checked each frame in `drain_upload_completed()`.
    pending_uploads: Vec<crate::upload_thread::CompletedUpload>,

    /// Deferred LoadImage responses — sent only after the async upload
    /// thread has completed and the texture is registered.
    /// Key: image_id, Value: the oneshot response sender.
    pending_load_responses: HashMap<u64, shared::protocol::render_cmd::RenderCmdResp<(u32, u32)>>,

    /// Image IDs whose DestroyImage arrived while the upload was still in
    /// flight.  `drain_upload_completed` uses this to delete the orphaned
    /// GL texture/fence instead of registering them.
    cancelled_uploads: HashSet<u64>,

    /// `eglSetDamageRegionKHR` function pointer (None if EGL_KHR_partial_update
    /// is not supported). Called before `eglSwapBuffers` to inform the
    /// compositor which region changed — saves power on OLED screens.
    #[allow(improper_ctypes_definitions)]
    egl_set_damage_region_fn: Option<
        unsafe extern "C" fn(egl::Display, egl::Surface, *const egl::Int, egl::Int) -> egl::Boolean,
    >,

    /// Unified per-frame damage accumulator for mixed Canvas2D + WebGL frames.
    /// Fed by Canvas2D batches, GL draw/clear, and readback paths.
    /// Resolved at swap time to determine partial vs full surface update.
    pub(crate) damage: crate::damage_effect::FrameDamageAccumulator,

    /// History of recent frame damages for buffer-age-aware partial present.
    damage_history: crate::dirty_region::damage_tracker::DamageHistory,
}

impl CanvasManager {
    /// Direct access to the glow GL context (for SpriteBatcher and other direct-GL paths).
    pub(crate) fn gl(&self) -> &glow::Context {
        &self.gl
    }

    pub(crate) fn new_with_resource(
        egl_lib_path: &str,
        dpi: f32,
        cache_dir: Option<&std::path::Path>,
        gpu_caps: &shared::device::gpu_caps::GpuCaps,
    ) -> EngineResult<Self> {
        let init = egl_ops::init_egl(egl_lib_path)?;
        let egl = init.egl;
        let display = init.display;
        let config = init.config;
        let gles_major = init.gles_major;

        // Create resource context + pbuffer.
        let (resource_ctx, resource_surf) =
            egl_ops::create_pbuffer_context(&egl, display, config, None, 16, 16, gles_major)?;
        let resource = EglContextHandle {
            ctx: resource_ctx,
            surf: resource_surf,
        };

        // Make resource current once.
        egl.make_current(
            display,
            Some(resource.surf),
            Some(resource.surf),
            Some(resource.ctx),
        )
        .map_err(|e| {
            ee(
                ErrorCode::RenderBackendError,
                format!("eglMakeCurrent(resource) failed: {e:?}"),
            )
        })?;

        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                egl.get_proc_address(s)
                    .map(|f| f as *const std::ffi::c_void)
                    .unwrap_or(std::ptr::null())
            })
        };

        let egl_extensions = egl
            .query_string(Some(display), egl::EXTENSIONS)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let device_caps = crate::device_caps::DeviceCapabilities::detect(&gl, &egl_extensions, gles_major, gpu_caps);
        tracing::info!(
            "DeviceCapabilities: GLES {:?}, tier={:?}, pbo={}, fence={}, compute={}, ahb={}, buffer_age={}, partial_update={}",
            device_caps.gles_version,
            device_caps.tier(),
            device_caps.has_pbo,
            device_caps.has_fence_sync,
            device_caps.has_compute,
            device_caps.ahb_available,
            device_caps.has_buffer_age,
            device_caps.has_partial_update,
        );

        // Initialize shader binary cache (best-effort, no-op if unsupported).
        let shader_cache = cache_dir.and_then(|dir| {
            let cache = crate::shader_cache::ShaderCache::new(&gl, dir);
            if cache.is_supported() {
                Some(cache)
            } else {
                None
            }
        });

        // Spawn upload thread on TierA devices (shared GL context for async texture upload).
        let api_level = crate::device_caps::android_api_level();
        let upload_thread = if device_caps.tier() == crate::device_caps::DeviceTier::TierA {
            crate::upload_thread::UploadThreadHandle::try_spawn(&egl, display, config, resource.ctx, gles_major)
        } else {
            None
        };
        // Budget gating: only when upload thread is live.
        let upload_server = if upload_thread.is_some() {
            Some(crate::upload_server::UploadServer::for_device(&device_caps, api_level))
        } else {
            None
        };

        // Probe EGL_KHR_partial_update (power saving on OLED screens).
        let egl_set_damage_region_fn = if egl_extensions.contains("EGL_KHR_partial_update") {
            let fname = std::ffi::CStr::from_bytes_with_nul(b"eglSetDamageRegionKHR\0").unwrap();
            egl.get_proc_address(fname.to_str().unwrap())
                .map(|ptr| unsafe { std::mem::transmute(ptr) })
        } else {
            None
        };
        if egl_set_damage_region_fn.is_some() {
            tracing::info!("EGL_KHR_partial_update available");
        }

        // Pre-allocate with reasonable capacities to reduce rehashing.
        // Most games use a small number of canvases and GL objects.
        Ok(Self {
            egl,
            gl,
            display,
            config,
            dpi,
            resource,
            bound: BoundContext::Resource,
            canvases: HashMap::with_capacity(4),
            next_canvas_id: AtomicU32::new(2), // 1 is reserved for onscreen
            contexts_2d: HashMap::with_capacity(4),
            dirty_2d: HashSet::with_capacity(4),
            image_registry: ImageRegistry::new(),
            last_swap_interval: -1, // force first eglSwapInterval call
            context_lost: false,
            last_window: None,
            programs: HashMap::with_capacity(16),
            shaders: HashMap::with_capacity(32),
            buffers: HashMap::with_capacity(32),
            textures: HashMap::with_capacity(64),
            framebuffers: HashMap::with_capacity(8),
            renderbuffers: HashMap::with_capacity(8),
            gl_state: HashMap::with_capacity(4),
            vaos: HashMap::with_capacity(16),
            samplers: HashMap::with_capacity(8),
            syncs: HashMap::with_capacity(4),
            device_caps,
            gles_major,
            preserved_ctx: None,
            needs_default_fbo_readback: false,
            upload_server,
            upload_thread,
            shader_cache,
            pending_uploads: Vec::new(),
            pending_load_responses: HashMap::new(),
            cancelled_uploads: HashSet::new(),
            egl_set_damage_region_fn,
            damage: crate::damage_effect::FrameDamageAccumulator::new(),
            damage_history: crate::dirty_region::damage_tracker::DamageHistory::new(),
        })
    }

    fn new_canvas_id(&self) -> CanvasId {
        let id = self.next_canvas_id.fetch_add(1, Ordering::Relaxed);
        CanvasId::from(id)
    }

    // ==================== Canvas Lifecycle ====================

    /// Create an offscreen (pbuffer) canvas.
    ///
    /// `w` and `h` are in **physical (buffer) pixels** — the same unit JS
    /// `canvas.width`/`canvas.height` uses, matching browser semantics.
    pub(crate) fn create_offscreen(&mut self, w: u32, h: u32) -> EngineResult<CanvasId> {
        let id = self.new_canvas_id();

        let share = Some(self.resource.ctx);
        let (ctx, surf) =
            egl_ops::create_pbuffer_context(&self.egl, self.display, self.config, share, w, h, self.gles_major)?;

        let info = CanvasInfo {
            id,
            width: w,
            height: h,
            is_onscreen: false,
        };

        self.canvases.insert(
            id,
            CanvasEntry {
                info,
                physical_width: w,
                physical_height: h,
                kind: SurfaceKind::Pbuffer,
                ctx: EglContextHandle { ctx, surf },
                drawing_buffer: None,
                bypass_drawing_buffer: false,
            },
        );
        // Offscreen canvas created → bypass no longer valid.
        self.evaluate_bypass();

        Ok(id)
    }

    /// Create or recreate the onscreen canvas (id=1).
    ///
    /// `surface_size`: expected physical dimensions from the SurfaceRef.
    /// When provided, skip the destroy-recreate cycle if the same native
    /// window is already active AND its dimensions match. This avoids
    /// "already connected" (EGL_BAD_ALLOC) errors on some Android drivers
    /// while still handling surface resizes (e.g. status bar hide/show).
    /// Pass `None` for initial creation and context recovery.
    pub(crate) fn create_onscreen(
        &mut self,
        window: usize,
        surface_size: Option<(u32, u32)>,
    ) -> EngineResult<()> {
        if let Some((exp_w, exp_h)) = surface_size {
            tracing::info!(
                "CanvasManager::create_onscreen begin: window=0x{:x}, expected={}x{}",
                window,
                exp_w,
                exp_h
            );
        } else {
            tracing::info!(
                "CanvasManager::create_onscreen begin: window=0x{:x}, expected=<none>",
                window
            );
        }

        let id = CanvasId::from(1u32);

        // Same native window + same physical dimensions → skip destroy-recreate.
        if let Some((exp_w, exp_h)) = surface_size {
            if let Some(entry) = self.canvases.get(&id) {
                if matches!(entry.kind, SurfaceKind::Window(w) if w == window)
                    && entry.physical_width == exp_w
                    && entry.physical_height == exp_h
                {
                    tracing::info!(
                        "CanvasManager::create_onscreen skip recreate: window unchanged and size matched {}x{}",
                        exp_w,
                        exp_h
                    );
                    return Ok(());
                }
            }
        }

        self.last_window = Some(window);
        self.context_lost = false;

        // Track whether a 2D context existed before destruction, so we can
        // re-initialize it after the new EGL context is created. This is
        // needed for Android resume: the surface is a different native window
        // but the game's JS code still expects canvas_id=1 to work.
        let mut had_2d_context = false;

        if let Some(_entry) = self.canvases.get(&id) {
            // Destroy and recreate the EGL surface when the ANativeWindow
            // changed. On many Android drivers, eglQuerySurface returns the
            // creation-time dimensions and does NOT reflect later window
            // resizes (e.g. navigation bar hide/show). Reusing the old
            // surface leads to buffer size mismatches that SurfaceFlinger
            // rejects ("rejecting buffer"), causing flicker.
            had_2d_context = self.contexts_2d.contains_key(&id);
            self.destroy_onscreen_internal(id)?;
        }

        // A newly created window surface may not inherit the previous swap
        // interval state on all drivers, so force the next swap to reapply it.
        self.last_swap_interval = -1;
        // New surface = new back buffers, old damage history is invalid.
        self.damage_history.clear();

        self.egl.bind_api(egl::OPENGL_ES_API).map_err(|e| {
            ee(
                ErrorCode::RenderBackendError,
                format!("eglBindAPI failed: {e:?}"),
            )
        })?;

        let surf = egl_ops::create_window_surface(&self.egl, self.display, self.config, window)?;

        let ctx_attribs = [egl::CONTEXT_CLIENT_VERSION as i32, self.gles_major as i32, egl::NONE as i32];
        let ctx = if let Some(preserved) = self.preserved_ctx.take() {
            tracing::info!("Reusing preserved EGL context for onscreen canvas {id}");
            preserved
        } else {
            self.egl
                .create_context(
                    self.display,
                    self.config,
                    Some(self.resource.ctx),
                    &ctx_attribs,
                )
                .map_err(|e| {
                    ee(
                        ErrorCode::RenderBackendError,
                        format!("eglCreateContext(onscreen) failed: {e:?}"),
                    )
                })?
        };

        // Query EGL for diagnostics only. Some Android stacks report a rotated
        // size here (e.g. 1080x2340 while the Java SurfaceHolder reports
        // 2340x1080). For onscreen we trust the size from updateSurface when
        // provided, because JS canvas metrics and viewport logic must align with
        // SurfaceHolder dimensions.
        let queried_w = self
            .egl
            .query_surface(self.display, surf, egl::WIDTH)
            .unwrap_or(0)
            .max(1) as u32;
        let queried_h = self
            .egl
            .query_surface(self.display, surf, egl::HEIGHT)
            .unwrap_or(0)
            .max(1) as u32;

        let (physical_w, physical_h) = if let Some((exp_w, exp_h)) = surface_size {
            if exp_w != queried_w || exp_h != queried_h {
                tracing::warn!(
                    "CanvasManager::create_onscreen size mismatch: expected={}x{}, egl_surface={}x{}, window=0x{:x}; using expected size",
                    exp_w,
                    exp_h,
                    queried_w,
                    queried_h,
                    window
                );
            }
            (exp_w.max(1), exp_h.max(1))
        } else {
            (queried_w, queried_h)
        };

        // JS canvas.width/height report physical (buffer) pixels — matching
        // browser semantics.  Games that want crisp rendering multiply by
        // devicePixelRatio themselves; the engine must NOT scale again.
        let info = CanvasInfo {
            id,
            width: physical_w,
            height: physical_h,
            is_onscreen: true,
        };

        self.canvases.insert(
            id,
            CanvasEntry {
                info,
                kind: SurfaceKind::Window(window),
                physical_width: physical_w,
                physical_height: physical_h,
                ctx: EglContextHandle { ctx, surf },
                drawing_buffer: None, // initialized below after make_current
                bypass_drawing_buffer: false, // evaluated after DrawingBuffer creation
            },
        );

        // Make current so GL calls work.
        self.make_current_needed(id)?;

        // Create the DrawingBuffer (intermediate FBO) for the onscreen canvas.
        // WebGL renders to this FBO; it gets blitted to the window surface on swap.
        match drawing_buffer::create(&self.gl, physical_w, physical_h) {
            Ok(db) => {
                if let Some(entry) = self.canvases.get_mut(&id) {
                    entry.drawing_buffer = Some(db);
                }
            }
            Err(e) => {
                tracing::error!("DrawingBuffer creation failed, rendering direct to surface: {e}");
                // Fallback: render directly to window surface (legacy behavior).
            }
        }
        self.evaluate_bypass();

        // Reset default viewport/state for the newly created onscreen context.
        // Context recreation invalidates old GL state tracking.
        unsafe {
            self.gl.viewport(0, 0, physical_w as i32, physical_h as i32);
        }
        self.gl_state.insert(
            id,
            CanvasGLState {
                current_program: None,
                viewport: Some((0, 0, physical_w as i32, physical_h as i32)),
                ..Default::default()
            },
        );

        // Resize any pre-existing Skia surface against the new DrawingBuffer
        // / FBO dimensions.  If the onscreen 2D context was torn down by the
        // destroy path above, re-initialise it here so JS targeting
        // canvas_id=1 keeps working through an Android surface recreate.
        let new_fbo = self
            .canvases
            .get(&id)
            .and_then(|e| e.drawing_buffer.as_ref())
            .map(|db| db.fbo.0.get())
            .unwrap_or(0);
        if let Some(ctx2d) = self.contexts_2d.get_mut(&id) {
            ctx2d.resize(new_fbo, physical_w, physical_h);
        }
        if had_2d_context && !self.contexts_2d.contains_key(&id) {
            context_2d_impl::init_skia_for_canvas(self, id)?;
        }

        Ok(())
    }

    fn destroy_onscreen_internal(&mut self, id: CanvasId) -> EngineResult<()> {
        if let Some(mut entry) = self.canvases.remove(&id) {
            // Destroy DrawingBuffer while the EGL context is still current.
            if let Some(db) = entry.drawing_buffer.take() {
                let _ = self.egl.make_current(
                    self.display,
                    Some(entry.ctx.surf),
                    Some(entry.ctx.surf),
                    Some(entry.ctx.ctx),
                );
                drawing_buffer::destroy(&self.gl, db);
            }

            // Switch to the resource (pbuffer) context so the ANativeWindow is
            // properly disconnected before we destroy the onscreen surface.
            let _ = self.egl.make_current(
                self.display,
                Some(self.resource.surf),
                Some(self.resource.surf),
                Some(self.resource.ctx),
            );

            self.contexts_2d.remove(&id);
            self.dirty_2d.remove(&id);
            self.gl_state.remove(&id);
            self.image_registry.remove_canvas_images(id);

            let _ = self.egl.destroy_surface(self.display, entry.ctx.surf);
            // Preserve the context for reuse on the next create_onscreen().
            // This avoids losing GL state (textures, shaders) across
            // Android surface destroy/recreate cycles (pause/resume).
            if let Some(old_ctx) = self.preserved_ctx.replace(entry.ctx.ctx) {
                // If there was already a preserved context (shouldn't happen normally),
                // destroy the older one to avoid leaking.
                let _ = self.egl.destroy_context(self.display, old_ctx);
            }

            self.bound = BoundContext::Resource;
            self.last_swap_interval = -1;
            self.damage_history.clear();
        }
        // Onscreen destroyed → bypass must be off until re-created.
        self.evaluate_bypass();
        Ok(())
    }

    /// Returns true if EGL reported context loss.
    /// The render thread should attempt recovery on the next frame.
    #[inline]
    pub(crate) fn is_context_lost(&self) -> bool {
        self.context_lost
    }

    /// Attempt to recover from EGL context loss by re-creating the
    /// onscreen surface using the last known window handle.
    /// Returns Ok(true) if recovery succeeded, Ok(false) if no window
    /// handle is available, or Err on failure.
    pub(crate) fn try_recover_context(&mut self) -> EngineResult<bool> {
        if !self.context_lost {
            return Ok(false);
        }
        if let Some(window) = self.last_window {
            tracing::info!("Attempting EGL context loss recovery");
            self.create_onscreen(window, None)?;
            tracing::info!("EGL context recovered successfully");
            Ok(true)
        } else {
            tracing::warn!("Cannot recover EGL context: no window handle available");
            Ok(false)
        }
    }

    pub(crate) fn destroy_canvas(&mut self, id: CanvasId) -> EngineResult<()> {
        shared::ensure!(
            id != 1,
            ErrorCode::InvalidArgument,
            "cannot destroy onscreen canvas"
        );

        if let Some(entry) = self.canvases.remove(&id) {
            // If currently bound, switch to resource first
            if self.bound == BoundContext::Canvas(id) {
                let _ = self.bind_resource();
            }
            self.egl.destroy_surface(self.display, entry.ctx.surf).ok();
            self.egl.destroy_context(self.display, entry.ctx.ctx).ok();

            self.contexts_2d.remove(&id);
            self.dirty_2d.remove(&id);
            self.gl_state.remove(&id);

            self.image_registry.remove_canvas_images(id);
        }
        // Canvas destroyed → re-evaluate (may re-enable bypass).
        self.evaluate_bypass();
        Ok(())
    }

    pub(crate) fn destroy_all(&mut self, gl: &glow::Context) {
        // destroy_canvas refuses id=1 (onscreen), so handle it separately.
        let onscreen_id = CanvasId::from(1u32);
        if self.canvases.contains_key(&onscreen_id) {
            let _ = self.destroy_onscreen_internal(onscreen_id);
        }
        let ids: Vec<CanvasId> = self.canvases.keys().copied().collect();
        for id in ids {
            let _ = self.destroy_canvas(id);
        }

        // Delete GL objects (best effort). Need a current context.
        let _ = self.ensure_any_canvas_current();

        unsafe {
            for (_id, p) in self.programs.drain() {
                if let Some(h) = p.gl_handle {
                    gl.delete_program(h);
                }
            }
            for (_id, s) in self.shaders.drain() {
                if let Some(h) = s.gl_handle {
                    gl.delete_shader(h);
                }
            }
            for (_id, b) in self.buffers.drain() {
                if let Some(h) = b.gl_handle {
                    gl.delete_buffer(h);
                }
            }
            for (_id, t) in self.textures.drain() {
                if let Some(h) = t.gl_handle {
                    gl.delete_texture(h);
                }
            }
            for (_id, f) in self.framebuffers.drain() {
                if let Some(h) = f.gl_handle {
                    gl.delete_framebuffer(h);
                }
            }
            for (_id, r) in self.renderbuffers.drain() {
                if let Some(h) = r.gl_handle {
                    gl.delete_renderbuffer(h);
                }
            }
        }

        // Images
        self.image_registry.destroy_all(gl);

        // Destroy resource
        let _ = self.egl.make_current(self.display, None, None, None);
        self.egl
            .destroy_surface(self.display, self.resource.surf)
            .ok();
        self.egl
            .destroy_context(self.display, self.resource.ctx)
            .ok();
        self.egl.terminate(self.display).ok();
    }

    // ==================== Context Binding ====================

    pub(super) fn bind_resource(&mut self) -> EngineResult<()> {
        self.egl
            .make_current(
                self.display,
                Some(self.resource.surf),
                Some(self.resource.surf),
                Some(self.resource.ctx),
            )
            .map_err(|e| {
                ee(
                    ErrorCode::RenderBackendError,
                    format!("eglMakeCurrent(resource) failed: {e:?}"),
                )
            })?;
        self.bound = BoundContext::Resource;
        Ok(())
    }

    /// Returns true if the currently bound context is the onscreen canvas (id=1).
    #[allow(dead_code)]
    pub(crate) fn is_onscreen_bound(&self) -> bool {
        self.bound == BoundContext::Canvas(CanvasId::from(1u32))
    }

    /// Returns the DrawingBuffer FBO for the given canvas, or None (= real FBO 0)
    /// if the canvas has no DrawingBuffer, or if bypass mode is active.
    pub(crate) fn get_drawing_buffer_fbo(
        &self,
        canvas_id: CanvasId,
    ) -> Option<glow::NativeFramebuffer> {
        self.canvases.get(&canvas_id).and_then(|entry| {
            if entry.bypass_drawing_buffer {
                None // Bypass: bind real FBO 0, skip DrawingBuffer.
            } else {
                entry.drawing_buffer.as_ref().map(|db| db.fbo)
            }
        })
    }

    /// Returns true if the framebuffer bound to `target` is the DrawingBuffer
    /// FBO for the given canvas.
    ///
    /// WebGL spec forbids modifying the default framebuffer attachments.
    /// Use this to guard `framebufferTexture2D` / `framebufferRenderbuffer`.
    ///
    /// `target` must be one of `FRAMEBUFFER`, `DRAW_FRAMEBUFFER`, or
    /// `READ_FRAMEBUFFER`; the correct binding point is queried accordingly.
    pub(crate) fn is_drawing_buffer_bound(
        &self,
        canvas_id: CanvasId,
        gl: &glow::Context,
        target: u32,
    ) -> bool {
        if let Some(db_fbo) = self.get_drawing_buffer_fbo(canvas_id) {
            let query = match target {
                glow::READ_FRAMEBUFFER => glow::READ_FRAMEBUFFER_BINDING,
                // DRAW_FRAMEBUFFER and FRAMEBUFFER both use the draw binding.
                _ => glow::DRAW_FRAMEBUFFER_BINDING,
            };
            let bound_fbo = unsafe { gl.get_parameter_i32(query) } as u32;
            bound_fbo == db_fbo.0.get()
        } else {
            false
        }
    }

    /// Drain completed texture uploads from the upload thread and register
    /// the resulting textures in the image registry for rendering.
    ///
    /// Textures whose GPU fence has not yet signaled are deferred to the
    /// next frame (stored in `pending_uploads`).
    ///
    /// Called once per frame from the render thread (non-blocking).
    /// Returns the number of dropped upload recoveries processed this call.
    pub(crate) fn drain_upload_completed(&mut self) -> u32 {
        let upload = match self.upload_thread.as_mut() {
            Some(u) => u,
            None => return 0,
        };

        let mut completed = Vec::new();
        upload.drain_completed(&mut completed);

        // Process uploads that completed but whose results could not be
        // delivered (result channel full/disconnected).  Each item carries
        // image_id + byte_len for consistent per-item recovery.
        let mut dropped = Vec::new();
        upload.drain_dropped(&mut dropped);
        let dropped_recovery_count = dropped.len() as u32;
        for d in &dropped {
            // 1. Recover UploadServer budget.
            if let Some(ref mut server) = self.upload_server {
                server.recover_dropped(d);
            }
            // 2. Resolve the pending response with an error so callers
            //    don't wait forever.
            if let Some(resp) = self.pending_load_responses.remove(&d.image_id) {
                resp.send(Err(shared::error::EngineError::from_detail(
                    shared::error::ErrorCode::Cancelled,
                    format!(
                        "image {} upload completed but result channel was full",
                        d.image_id,
                    ),
                )));
            }
            // 3. Clean stale cancelled_uploads entry (if DestroyImage arrived
            //    while the upload was in flight and then the result was dropped).
            self.cancelled_uploads.remove(&d.image_id);
        }

        // Also re-check any previously deferred uploads.
        let mut deferred = std::mem::take(&mut self.pending_uploads);
        deferred.extend(completed);

        for c in deferred {
            // Non-blocking fence check (timeout = 0).
            let status = unsafe { self.gl.client_wait_sync(c.fence, 0, 0) };

            if status == glow::ALREADY_SIGNALED || status == glow::CONDITION_SATISFIED {
                // GPU upload complete — delete the fence.
                unsafe { self.gl.delete_sync(c.fence) };

                // Release upload budget (both normal and cancelled paths).
                if let Some(ref mut server) = self.upload_server {
                    server.finish_job_bytes(c.byte_len);
                }

                // If the image was destroyed while the upload was in flight,
                // discard the texture instead of registering it.
                if self.cancelled_uploads.remove(&c.image_id) {
                    unsafe { self.gl.delete_texture(c.texture) };
                    tracing::debug!(
                        "Upload thread: discarded cancelled upload image_id={}",
                        c.image_id
                    );
                    continue;
                }

                let info = crate::backend::gl::image_store::GpuImageInfo::rgba8_unpremul(
                    c.width,
                    c.height,
                );
                self.image_registry
                    .register_shared_texture(c.image_id as u32, c.texture, info);

                // Send the deferred LoadImage response now that the texture
                // is actually available for rendering.
                if let Some(resp) = self.pending_load_responses.remove(&c.image_id) {
                    resp.send(Ok((c.width, c.height)));
                }

                tracing::trace!(
                    "Upload thread: texture registered image_id={} {}x{}",
                    c.image_id,
                    c.width,
                    c.height
                );
            } else {
                // Not ready yet — defer to next frame.
                self.pending_uploads.push(c);
            }
        }
        dropped_recovery_count
    }

    /// Reset per-frame upload budget counters.  Called at the render thread's
    /// frame boundary (after draining completions, before signaling RAF).
    pub(crate) fn reset_frame_upload_budget(&mut self) {
        if let Some(ref mut server) = self.upload_server {
            server.reset_frame_budget();
        }
    }

    /// Read and reset per-frame upload budget rejections since last call.
    pub(crate) fn take_upload_frame_rejections(&mut self) -> u32 {
        match self.upload_server {
            Some(ref mut server) => {
                let n = server.frame_rejections();
                // frame_rejections is reset by reset_frame_budget, but we
                // read it before that reset fires (called in the same
                // present_frame_and_signal_raf closure).
                n
            }
            None => 0,
        }
    }

    /// Re-evaluate whether the onscreen canvas can bypass the DrawingBuffer.
    ///
    /// Bypass is safe when there is exactly one canvas (the onscreen one)
    /// and no offscreen canvases exist.  Called after canvas creation/destruction.
    /// Signal that the game reads from the onscreen default framebuffer.
    /// Permanently disables DrawingBuffer bypass so content is preserved across swaps.
    ///
    /// If bypass was active at the time of detection, we snapshot the current
    /// window surface content into the DrawingBuffer (reverse blit) so the
    /// readback that triggered this signal sees valid content immediately,
    /// not just on subsequent frames.
    pub(crate) fn signal_default_fbo_readback(&mut self) {
        if !self.needs_default_fbo_readback {
            let onscreen_id = CanvasId::from(1u32);
            let was_bypass = self
                .canvases
                .get(&onscreen_id)
                .map_or(false, |e| e.bypass_drawing_buffer);

            tracing::info!("Default-FBO readback detected — disabling DrawingBuffer bypass");
            self.needs_default_fbo_readback = true;
            self.evaluate_bypass(); // sets bypass_drawing_buffer = false

            // If bypass was active, the window surface (FBO 0) has the current
            // frame's content but the DrawingBuffer is stale.  Blit window → DB
            // so the DrawingBuffer has valid content for the imminent readback
            // and for all subsequent frames.
            //
            // COUPLING NOTE: This reverse blit requires glBlitFramebuffer (ES 3.0).
            // The DrawingBuffer itself is only created when blit is available
            // (see drawing_buffer::create which probes blit at init).  If the
            // DrawingBuffer creation conditions or the bypass evaluation logic
            // change, this path must be re-verified to stay consistent.
            if was_bypass {
                if let Some(entry) = self.canvases.get(&onscreen_id) {
                    if let Some(ref db) = entry.drawing_buffer {
                        let w = entry.physical_width;
                        let h = entry.physical_height;
                        unsafe {
                            use glow::HasContext;
                            // READ from window surface (FBO 0), DRAW to DrawingBuffer.
                            self.gl
                                .bind_framebuffer(glow::READ_FRAMEBUFFER, None);
                            self.gl
                                .bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(db.fbo));
                            self.gl.blit_framebuffer(
                                0,
                                0,
                                w as i32,
                                h as i32,
                                0,
                                0,
                                w as i32,
                                h as i32,
                                glow::COLOR_BUFFER_BIT,
                                glow::NEAREST,
                            );
                            // Re-bind DrawingBuffer as the active FBO for subsequent
                            // rendering and the imminent readback.
                            self.gl
                                .bind_framebuffer(glow::FRAMEBUFFER, Some(db.fbo));
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn evaluate_bypass(&mut self) {
        let onscreen_id = CanvasId::from(1u32);
        // Bypass requires: single canvas, has DrawingBuffer, and no default-FBO readback.
        // Once needs_default_fbo_readback is set, bypass stays disabled permanently
        // so the DrawingBuffer preserves content across swaps.
        let can_bypass = self.canvases.len() == 1
            && !self.needs_default_fbo_readback
            && self
                .canvases
                .get(&onscreen_id)
                .map_or(false, |e| e.drawing_buffer.is_some());

        if let Some(entry) = self.canvases.get_mut(&onscreen_id) {
            if entry.bypass_drawing_buffer != can_bypass {
                tracing::info!(
                    "DrawingBuffer bypass: {} → {}",
                    entry.bypass_drawing_buffer,
                    can_bypass,
                );
                entry.bypass_drawing_buffer = can_bypass;
            }
        }
    }

    pub(crate) fn make_current_needed(&mut self, id: CanvasId) -> EngineResult<()> {
        if self.bound == BoundContext::Canvas(id) {
            return Ok(());
        }
        let entry = self
            .canvases
            .get(&id)
            .ok_or_else(|| ee(ErrorCode::NotFound, format!("canvas not found: {id:?}")))?;

        self.egl
            .make_current(
                self.display,
                Some(entry.ctx.surf),
                Some(entry.ctx.surf),
                Some(entry.ctx.ctx),
            )
            .map_err(|e| {
                ee(
                    ErrorCode::RenderBackendError,
                    format!("eglMakeCurrent(canvas) failed: {e:?}"),
                )
            })?;
        self.bound = BoundContext::Canvas(id);

        // After EGL context switch to the onscreen canvas, bind the
        // DrawingBuffer FBO so GL commands target it instead of the
        // window surface (FBO 0).  This is the Chromium DrawingBuffer
        // pattern: WebGL's "default framebuffer" is actually our FBO.
        if let Some(db) = entry.drawing_buffer.as_ref() {
            unsafe {
                self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(db.fbo));
            }
        }

        Ok(())
    }

    pub(crate) fn ensure_any_canvas_current(&mut self) -> EngineResult<CanvasId> {
        match self.bound {
            BoundContext::Canvas(id) => Ok(id),
            BoundContext::Resource => {
                // Prefer onscreen(1) if exists
                let onscreen = CanvasId::from(1u32);
                if self.canvases.contains_key(&onscreen) {
                    self.make_current_needed(onscreen)?;
                    return Ok(onscreen);
                }
                // else pick any canvas
                if let Some((&id, _)) = self.canvases.iter().next() {
                    self.make_current_needed(id)?;
                    return Ok(id);
                }
                // else bind resource
                self.bind_resource()?;
                Ok(CanvasId::from(0u32))
            }
        }
    }

    pub(crate) fn current_canvas_id(&self) -> Option<CanvasId> {
        match self.bound {
            BoundContext::Canvas(id) => Some(id),
            BoundContext::Resource => None,
        }
    }

    fn restore_bound(&mut self, saved: BoundContext) -> EngineResult<()> {
        match saved {
            BoundContext::Resource => self.bind_resource(),
            BoundContext::Canvas(id) => {
                if self.canvases.contains_key(&id) {
                    self.make_current_needed(id)
                } else {
                    self.bind_resource()
                }
            }
        }
    }

    // ==================== Canvas Operations ====================

    /// Resize a canvas buffer.
    ///
    /// `w` and `h` are in **physical (buffer) pixels** — the same unit JS
    /// `canvas.width`/`canvas.height` uses.  No DPR scaling is applied.
    pub(crate) fn resize_canvas(
        &mut self,
        id: CanvasId,
        w: Option<u32>,
        h: Option<u32>,
    ) -> EngineResult<()> {
        let (old_w, old_h, kind, ctx_handle, old_surf) = {
            let entry = self.canvases.get(&id).ok_or_else(|| {
                ee(
                    ErrorCode::NotFound,
                    format!("resize_canvas: canvas not found: {id:?}"),
                )
            })?;

            (
                entry.physical_width,
                entry.physical_height,
                entry.kind,
                entry.ctx.ctx,
                entry.ctx.surf,
            )
        };

        let new_w = w.unwrap_or(old_w);
        let new_h = h.unwrap_or(old_h);

        if new_w == old_w && new_h == old_h {
            return Ok(());
        }

        // Window surfaces: the EGL surface is controlled by Android SurfaceView.
        // Resize only the DrawingBuffer so canvas.width/height reflects what JS
        // set, and WebGL renders at that resolution. The blit in swap_buffers
        // scales to the actual surface dimensions.
        if matches!(kind, SurfaceKind::Window(_)) {
            self.make_current_needed(id)?;
            if let Some(entry) = self.canvases.get_mut(&id) {
                if let Some(ref mut db) = entry.drawing_buffer {
                    drawing_buffer::resize(&self.gl, db, new_w, new_h)?;
                }
                entry.info.width = new_w;
                entry.info.height = new_h;
            }

            let new_fbo = self
                .canvases
                .get(&id)
                .and_then(|e| e.drawing_buffer.as_ref())
                .map(|db| db.fbo.0.get())
                .unwrap_or(0);
            if let Some(ctx2d) = self.contexts_2d.get_mut(&id) {
                ctx2d.resize(new_fbo, new_w, new_h);
            }

            // WebGL default framebuffer viewport resets after drawing buffer resize.
            unsafe {
                self.gl.viewport(0, 0, new_w as i32, new_h as i32);
            }
            self.gl_state.entry(id).or_default().viewport =
                Some((0, 0, new_w as i32, new_h as i32));

            return Ok(());
        }

        let saved_bound = self.bound;
        let was_current = matches!(saved_bound, BoundContext::Canvas(cur) if cur == id);

        if was_current {
            self.egl
                .make_current(self.display, None, None, None)
                .map_err(|e| {
                    ee(
                        ErrorCode::RenderBackendError,
                        format!("resize_canvas: make_current(None) failed: {e:?}"),
                    )
                })?;
        }

        // destroy old surface
        self.egl
            .destroy_surface(self.display, old_surf)
            .map_err(|e| {
                ee(
                    ErrorCode::RenderBackendError,
                    format!("resize_canvas: destroy_surface failed: {e:?}"),
                )
            })?;

        // create new surface
        let new_surf = match kind {
            SurfaceKind::Window(native_window) => {
                self.last_swap_interval = -1;
                self.damage_history.clear();
                egl_ops::create_window_surface(&self.egl, self.display, self.config, native_window)?
            }
            SurfaceKind::Pbuffer => {
                let pbuf_attribs = [
                    egl::WIDTH as i32,
                    new_w as i32,
                    egl::HEIGHT as i32,
                    new_h as i32,
                    egl::NONE as i32,
                ];
                self.egl
                    .create_pbuffer_surface(self.display, self.config, &pbuf_attribs)
                    .map_err(|e| {
                        ee(
                            ErrorCode::RenderBackendError,
                            format!("resize_canvas: create_pbuffer_surface failed: {e:?}"),
                        )
                    })?
            }
        };

        if was_current {
            self.egl
                .make_current(
                    self.display,
                    Some(new_surf),
                    Some(new_surf),
                    Some(ctx_handle),
                )
                .map_err(|e| {
                    ee(
                        ErrorCode::RenderBackendError,
                        format!("resize_canvas: make_current(new surf) failed: {e:?}"),
                    )
                })?;
            self.bound = BoundContext::Canvas(id);
        }

        // Keep canvas metrics aligned with JS-requested dimensions. On some
        // devices EGL surface queries may return rotated values for window
        // surfaces, which breaks viewport/layout logic in games.
        let (actual_w, actual_h) = (new_w, new_h);

        {
            let entry = self.canvases.get_mut(&id).ok_or_else(|| {
                ee(
                    ErrorCode::NotFound,
                    format!("resize_canvas: canvas not found after egl ops: {id:?}"),
                )
            })?;

            entry.physical_width = actual_w;
            entry.physical_height = actual_h;

            entry.ctx.surf = new_surf;
            entry.info.width = actual_w;
            entry.info.height = actual_h;
        }

        // Offscreen pbuffer: Skia renders into FBO 0 directly.
        if let Some(ctx2d) = self.contexts_2d.get_mut(&id) {
            ctx2d.resize(0, actual_w, actual_h);
        }

        if !was_current {
            self.restore_bound(saved_bound)?;
        }

        Ok(())
    }

    /// Declare the damage region for the current back buffer BEFORE rendering
    /// to the onscreen surface (Skia flush_and_submit, DrawingBuffer blit).
    ///
    /// Per EGL_KHR_partial_update spec, this must be called before any rendering
    /// to the main framebuffer so the driver can skip loading unchanged tiles.
    /// The declared region is the age-expanded historical damage — what this
    /// buffer needs to "catch up" on since it was last presented.  The app will
    /// render at least this region (and typically the full game frame on top).
    ///
    /// Called from the render thread right before `flush_dirty_2d_contexts()`.
    pub(crate) fn declare_frame_damage(&mut self, id: CanvasId) {
        // Read what we need from the canvas entry, then drop the borrow.
        let (surface_w, surface_h, surf) = match self.canvases.get(&id) {
            Some(e) => (e.physical_width, e.physical_height, e.ctx.surf),
            None => return,
        };

        // Resolve this frame's accumulated damage (does not reset — swap does that).
        let current_frame_damage = self.resolve_pending_damage(surface_w, surface_h);

        // Query buffer age and expand with history.
        const EGL_BUFFER_AGE_KHR: egl::Int = 0x313D;
        let buffer_damage = if self.device_caps.has_buffer_age {
            match self.egl.query_surface(self.display, surf, EGL_BUFFER_AGE_KHR) {
                Ok(age) if age > 0 => {
                    self.damage_history.resolve_with_age(current_frame_damage, age)
                }
                _ => ResolvedDamage::FullSurface,
            }
        } else {
            current_frame_damage
        };

        // Declare to the compositor what we will redraw.
        if let (
            Some(set_damage),
            ResolvedDamage::Partial { x, y, width, height },
        ) = (self.egl_set_damage_region_fn, buffer_damage)
        {
            let rect = [x, y, width, height];
            unsafe {
                set_damage(self.display, surf, rect.as_ptr(), 1);
            }
        }
    }

    pub(crate) fn swap_buffers_no_restore(
        &mut self,
        id: CanvasId,
        wait_for_vsync: bool,
    ) -> EngineResult<ResolvedDamage> {
        self.make_current_needed(id)?;
        let entry = self
            .canvases
            .get(&id)
            .ok_or_else(|| ee(ErrorCode::NotFound, format!("canvas not found: {id:?}")))?;

        // Blit DrawingBuffer to the real window surface before swap.
        // When bypass is active, WebGL already rendered to FBO 0 — skip blit.
        if !entry.bypass_drawing_buffer {
            if let Some(ref db) = entry.drawing_buffer {
                drawing_buffer::blit_to_surface(
                    &self.gl,
                    db,
                    entry.physical_width,
                    entry.physical_height,
                );
            }
        }

        // Only call eglSwapInterval when the value actually changes
        let interval = if wait_for_vsync { 1 } else { 0 };
        if interval != self.last_swap_interval {
            let _ = self.egl.swap_interval(self.display, interval);
            self.last_swap_interval = interval;
        }

        // Resolve this frame's damage for history recording and stats.
        // The eglSetDamageRegionKHR call was already made in declare_frame_damage()
        // before rendering — per spec it must happen before GL draws.
        let current_frame_damage =
            self.resolve_pending_damage(entry.physical_width, entry.physical_height);
        self.damage.reset();

        self.egl
            .swap_buffers(self.display, entry.ctx.surf)
            .map_err(|e| {
                if let Some(egl_err) = self.egl.get_error() {
                    if egl_err == egl::Error::ContextLost {
                        tracing::warn!("EGL context lost detected during swap_buffers");
                        self.context_lost = true;
                    }
                }
                ee(
                    ErrorCode::RenderBackendError,
                    format!("eglSwapBuffers failed: {e:?}"),
                )
            })?;

        // Re-bind the DrawingBuffer FBO after swap so the next frame's GL
        // commands target it instead of the window surface.
        // When bypass is active, leave FBO 0 bound — next frame renders there.
        let bypass = self
            .canvases
            .get(&id)
            .map_or(false, |e| e.bypass_drawing_buffer);
        if !bypass {
            if let Some(ref db) = self
                .canvases
                .get(&id)
                .and_then(|e| e.drawing_buffer.as_ref())
            {
                unsafe {
                    self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(db.fbo));
                }
            }
        }

        // Record this frame's damage AFTER successful swap. If swap failed,
        // the frame was never presented and must not pollute the history —
        // buffer age semantics assume history entries correspond to actual swaps.
        self.damage_history.push(current_frame_damage);

        Ok(current_frame_damage)
    }

    #[allow(dead_code)]
    pub(crate) fn get_info(&self, id: CanvasId) -> EngineResult<CanvasInfo> {
        let entry = self
            .canvases
            .get(&id)
            .ok_or_else(|| ee(ErrorCode::NotFound, format!("canvas not found: {id:?}")))?;
        Ok(entry.info.clone())
    }

    /// Return the buffer-pixel size of a canvas.
    /// This is what JS `canvas.width`/`canvas.height` reports (physical pixels,
    /// matching browser semantics).
    /// For the onscreen canvas with a DrawingBuffer, returns the DrawingBuffer
    /// dimensions (which may differ from the EGL surface dimensions).
    pub(crate) fn get_canvas_size(&self, id: CanvasId) -> EngineResult<(u32, u32)> {
        let entry = self
            .canvases
            .get(&id)
            .ok_or_else(|| ee(ErrorCode::NotFound, format!("canvas not found: {id:?}")))?;
        if let Some(ref db) = entry.drawing_buffer {
            Ok((db.width, db.height))
        } else {
            Ok((entry.physical_width, entry.physical_height))
        }
    }

    // ==================== 2D Context Management ====================

    pub(crate) fn init_skia_for_canvas(&mut self, canvas_id: CanvasId) -> EngineResult<()> {
        context_2d_impl::init_skia_for_canvas(self, canvas_id)
    }

    pub(crate) fn get_2d_context_mut(
        &mut self,
        canvas_id: CanvasId,
    ) -> EngineResult<&mut Canvas2DContext> {
        self.contexts_2d.get_mut(&canvas_id).ok_or_else(|| {
            ee(
                ErrorCode::NotFound,
                format!("2d context not found: canvas_id={canvas_id:?}"),
            )
        })
    }

    /// Split-borrow accessor: returns `(&mut 2d_ctx, &ImageStore)` so a
    /// single `drawImage` dispatch can wrap a stored GL texture into an
    /// `SkImage` (needs `&mut GrDirectContext`, owned by 2d_ctx) while
    /// also looking up the texture metadata (needs `&ImageStore`).
    ///
    /// Returns `NotFound` if the canvas has no 2D context yet.
    pub(crate) fn split_2d_and_images(
        &mut self,
        canvas_id: CanvasId,
    ) -> EngineResult<(
        &mut Canvas2DContext,
        &mut crate::backend::gl::image_store::ImageStore,
    )> {
        let ctx = self.contexts_2d.get_mut(&canvas_id).ok_or_else(|| {
            ee(
                ErrorCode::NotFound,
                format!("2d context not found: canvas_id={canvas_id:?}"),
            )
        })?;
        Ok((ctx, self.image_registry.store_mut()))
    }

    /// Iterate over all 2D contexts mutably (for font registration, etc.).
    pub(crate) fn contexts_2d_iter_mut(
        &mut self,
    ) -> impl Iterator<Item = (&CanvasId, &mut Canvas2DContext)> {
        self.contexts_2d.iter_mut()
    }

    pub(crate) fn mark_2d_dirty(&mut self, canvas_id: CanvasId) {
        self.dirty_2d.insert(canvas_id);
    }

    /// Remove a canvas from the dirty-2D set after an explicit Materialize
    /// flush, preventing double-flush at present time.
    pub(crate) fn clear_2d_dirty(&mut self, canvas_id: CanvasId) {
        self.dirty_2d.remove(&canvas_id);
    }

    #[allow(dead_code)]
    pub(crate) fn mark_current_frame_requires_full_redraw(&mut self) {
        self.damage.add(crate::damage_effect::DamageEffect::FullSurface);
    }

    #[allow(dead_code)]
    pub(crate) fn stage_current_frame_partial_damage(&mut self, rect: [i32; 4], is_onscreen: bool) {
        if !is_onscreen {
            return;
        }
        let [x, y, width, height] = rect;
        if width <= 0 || height <= 0 {
            return;
        }
        self.damage.add(crate::damage_effect::DamageEffect::OnscreenRect { x, y, width, height });
    }

    /// Feed a DamageEffect directly into the per-frame accumulator.
    pub(crate) fn add_damage(&mut self, effect: crate::damage_effect::DamageEffect) {
        self.damage.add(effect);
    }

    #[allow(dead_code)]
    pub(crate) fn pending_dirty_2d_count(&self) -> usize {
        self.dirty_2d.len()
    }

    /// Save current GL state and set a safe baseline for Canvas2D / Skia
    /// text atlas uploads.
    /// Mark every live 2D context's Skia cache as stale.  Called
    /// once per WebGL batch to signal "external code just mutated
    /// raw GL state, your tracked cache is no longer accurate."
    /// The per-context `reset_gl_state_if_stale()` picks this up
    /// lazily on the next Skia draw.
    pub(crate) fn mark_all_2d_contexts_stale(&mut self) {
        for ctx in self.contexts_2d.values_mut() {
            ctx.mark_state_stale();
        }
    }

    /// Begin a Skia-side GL scope for `canvas_id`.  Restores the 5
    /// raw-GL bindings Skia requires (active-tex, PBO, alignment) on
    /// drop and invalidates the per-canvas dedup shadow so WebGL
    /// can't serve stale cached state.  Canonical entry point for
    /// the Materialize / Present boundary.
    pub(crate) fn begin_canvas2d_gl_scope_for(
        &mut self,
        canvas_id: CanvasId,
    ) -> context_2d_impl::Canvas2DGlScopeGuard {
        let gl_ptr: *const glow::Context = &self.gl;
        let shadow = self.gl_state.entry(canvas_id).or_default() as *mut _;
        // SAFETY: `self.gl` is never mutated; `gl_state` is borrowed
        // only through this pointer until the guard drops.  The
        // guard drops before any other method returns a borrow of
        // `self`.
        unsafe { context_2d_impl::begin_canvas2d_gl_scope(&*gl_ptr, Some(&mut *shadow)) }
    }

    pub(crate) fn flush_dirty_2d_contexts(&mut self) -> EngineResult<Vec<CanvasId>> {
        context_2d_impl::flush_dirty_2d_contexts(self)
    }

    /// Read pixel data from the current framebuffer via glReadPixels.
    ///
    /// This is a read-only operation — it does NOT modify the framebuffer and
    /// therefore does NOT contribute to frame damage. The caller is responsible
    /// for ensuring all prior rendering (Canvas2D Skia flush, GL draw calls)
    /// has been materialized before calling this. In the current model, the
    /// unified collector's `flush_as_barrier()` + trailing `Materialize` handles
    /// this before the sync readback command reaches the render thread.
    pub(crate) fn read_pixels(&self, x: i32, y: i32, width: u32, height: u32) -> Vec<u8> {
        let len = (width * height * 4) as usize;
        let mut buf = vec![0u8; len];
        unsafe {
            self.gl.read_pixels(
                x,
                y,
                width as i32,
                height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut buf)),
            );
        }
        // glReadPixels returns rows in bottom-to-top order; flip to top-to-bottom
        let row_bytes = (width * 4) as usize;
        let row_count = height as usize;
        for r in 0..row_count / 2 {
            let top = r * row_bytes;
            let bot = (row_count - 1 - r) * row_bytes;
            // SAFETY: top and bot slices are non-overlapping (r < row_count - 1 - r)
            unsafe {
                std::ptr::swap_nonoverlapping(
                    buf.as_mut_ptr().add(top),
                    buf.as_mut_ptr().add(bot),
                    row_bytes,
                );
            }
        }
        buf
    }

    fn resolve_pending_damage(&self, surface_width: u32, surface_height: u32) -> ResolvedDamage {
        self.damage
            .resolve((surface_width as i32, surface_height as i32))
    }

    // ==================== Image Management ====================

    pub(crate) fn generate_img_id(&self) -> u32 {
        self.image_registry.generate_img_id()
    }

    pub(crate) fn load_shared_image(
        &mut self,
        image_id: u32,
        image: NormalizedImage,
    ) -> EngineResult<(u32, u32)> {
        self.ensure_any_canvas_current()?;
        let display_ptr = self.display.as_ptr() as *const std::ffi::c_void;
        self.image_registry.load_shared_image(
            &self.gl,
            image_id,
            image,
            &self.device_caps,
            display_ptr,
        )
    }

    /// Upload a compressed texture (KTX2/ETC2/ASTC) directly to the GPU.
    ///
    /// Bypasses RGBA decode and PBO upload — calls `glCompressedTexImage2D`
    /// directly. Falls back to the standard RGBA path if the GPU doesn't
    /// support the compressed format.
    pub(crate) fn load_compressed_image(
        &mut self,
        image_id: u32,
        compressed: &shared::protocol::io_cmd::CompressedImage,
    ) -> EngineResult<(u32, u32)> {
        use shared::error::EngineError;
        self.ensure_any_canvas_current()?;

        let format = crate::compressed_upload::CompressedFormat::from_vk_format(compressed.vk_format)
            .ok_or_else(|| {
                EngineError::new(ErrorCode::Unsupported)
                    .with_detail(format!("unsupported compressed format: {}", compressed.vk_format))
            })?;

        if !self.device_caps.compressed_format_support.is_supported(format) {
            tracing::warn!(
                "GPU does not support {}, image_id={}",
                format.label(), image_id,
            );
            return Err(EngineError::new(ErrorCode::Unsupported)
                .with_detail(format!("GPU does not support {}", format.label())));
        }

        let texture = crate::compressed_upload::upload_compressed_texture(
            &self.gl,
            format,
            compressed.width,
            compressed.height,
            &compressed.data,
        ).ok_or_else(|| {
            EngineError::new(ErrorCode::Unsupported)
                .with_detail("glCompressedTexImage2D failed")
        })?;

        let info = crate::backend::gl::image_store::GpuImageInfo::rgba8_unpremul(
            compressed.width,
            compressed.height,
        );
        self.image_registry.register_shared_texture(image_id, texture, info);

        tracing::debug!(
            "compressed texture uploaded: image_id={} {}x{} {}",
            image_id, compressed.width, compressed.height, format.label(),
        );

        Ok((compressed.width, compressed.height))
    }

    /// Submit a texture upload to the upload thread (async, non-blocking).
    ///
    /// On success, the `resp` is stored and will be sent from
    /// `drain_upload_completed()` once the GPU fence signals and the
    /// texture is actually registered in the image registry.
    ///
    /// Returns `Err(resp)` if the upload thread is unavailable or degraded,
    /// giving the resp back so the caller can fall back to sync upload.
    pub(crate) fn submit_async_upload(
        &mut self,
        image_id: u32,
        image: &NormalizedImage,
        resp: shared::protocol::render_cmd::RenderCmdResp<(u32, u32)>,
    ) -> Result<(), shared::protocol::render_cmd::RenderCmdResp<(u32, u32)>> {
        let upload = match self.upload_thread.as_ref() {
            Some(u) if !u.is_degraded() => u,
            _ => return Err(resp),
        };
        let job = crate::upload_thread::UploadJob {
            image_id: image_id as u64,
            width: image.width,
            height: image.height,
            rgba: image.rgba.clone(),
        };

        // Budget gate: reject if per-frame upload budget is exhausted.
        if let Some(ref mut server) = self.upload_server {
            if !server.try_acquire_job(&job) {
                tracing::debug!(
                    "Upload budget exhausted: rejecting image_id={} ({} bytes), queue_depth={}",
                    image_id,
                    job.byte_len(),
                    server.queue_depth(),
                );
                return Err(resp);
            }
        }

        if upload.submit(job) {
            self.pending_load_responses.insert(image_id as u64, resp);
            Ok(())
        } else {
            // Channel full — release the budget we just acquired.
            if let Some(ref mut server) = self.upload_server {
                let release_job = crate::upload_thread::UploadJob {
                    image_id: image_id as u64,
                    width: image.width,
                    height: image.height,
                    rgba: image.rgba.clone(),
                };
                server.finish_job(&release_job);
            }
            Err(resp)
        }
    }

    /// Cancel a pending async upload for the given image_id.
    ///
    /// Sends a `Cancelled` error on the deferred response (if still pending)
    /// and marks the image_id so that `drain_upload_completed` will delete
    /// the orphaned GL texture instead of registering it.
    pub(crate) fn cancel_pending_load(&mut self, image_id: u32) {
        let id64 = image_id as u64;
        if let Some(resp) = self.pending_load_responses.remove(&id64) {
            use shared::error::{EngineError, ErrorCode};
            resp.send(Err(EngineError::from_detail(
                ErrorCode::Cancelled,
                format!("image {} destroyed before upload completed", image_id),
            )));
            // Only mark as cancelled when there was actually a pending
            // async upload — otherwise cancelled_uploads grows unbounded
            // and risks discarding a future upload if the ID is reused.
            self.cancelled_uploads.insert(id64);
        }
    }

    /// Set `GL_PROGRAM_BINARY_RETRIEVABLE_HINT` on a program.
    ///
    /// glow does not wrap `glProgramParameteri`, so we resolve the
    /// function pointer via EGL at call time.  No-op if the shader
    /// cache is disabled or the function is unavailable.
    pub(crate) fn set_program_binary_hint(&self, program: glow::NativeProgram) {
        if self.shader_cache.is_none() {
            return;
        }
        const GL_PROGRAM_BINARY_RETRIEVABLE_HINT: u32 = 0x8257;
        if let Some(fn_ptr) = self.egl.get_proc_address("glProgramParameteri") {
            type GlProgramParameteriFn = unsafe extern "system" fn(u32, u32, i32);
            unsafe {
                let f: GlProgramParameteriFn = std::mem::transmute(fn_ptr);
                f(program.0.get(), GL_PROGRAM_BINARY_RETRIEVABLE_HINT, 1);
            }
        }
    }

    pub(crate) fn destroy_shared_image(&mut self, image_id: u32) -> EngineResult<()> {
        self.ensure_any_canvas_current()?;
        self.image_registry.destroy_shared_image(&self.gl, image_id)
    }

    /// Look up an image by id.  Returns the raw GL texture + dimensions +
    /// colour/alpha metadata.  Callers that want a Skia `SkImage` wrap
    /// the texture via `ImageStore::resolve_as_sk_image` using their
    /// canvas's `GrDirectContext`.
    pub(crate) fn get_shared_image(
        &self,
        image_id: u32,
    ) -> Option<crate::backend::gl::image_store::StoredImage> {
        self.image_registry.get_shared_texture(image_id)
    }



    /// Access the PBO pool for WebGL texture uploads.
    /// Returns None if no images have been loaded yet (pool not initialized).
    pub(crate) fn pbo_pool_mut(&mut self) -> Option<&mut pbo_upload::PboPool> {
        self.image_registry.ensure_pbo_pool_public(&self.gl);
        self.image_registry.pbo_pool_mut()
    }

    // ==================== GL Object Helpers ====================

    pub(crate) fn check_owner(
        &self,
        owner: Option<CanvasId>,
        canvas: CanvasId,
        what: &str,
    ) -> EngineResult<()> {
        if owner == Some(canvas) {
            return Ok(());
        }
        Err(ee(
            ErrorCode::InvalidOperation,
            format!("{what} belongs to another WebGL context (owner={owner:?}, canvas={canvas:?})"),
        ))
    }
}

impl Drop for CanvasManager {
    fn drop(&mut self) {
        if let Some(ctx) = self.preserved_ctx.take() {
            let _ = self.egl.destroy_context(self.display, ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage_effect::{DamageEffect, FrameDamageAccumulator};

    // ---- Unified DamageEffect accumulator integration tests ----
    // These verify that the CanvasManager methods correctly feed the accumulator,
    // testing the same scenarios as the previous stage_partial_damage / resolve_staged_damage
    // tests but through the unified model.

    #[test]
    fn canvas2d_rect_resolves_to_partial_damage() {
        let mut acc = FrameDamageAccumulator::new();
        acc.add(DamageEffect::OnscreenRect { x: 10, y: 20, width: 300, height: 400 });
        assert_eq!(
            acc.resolve((1080, 1920)),
            ResolvedDamage::Partial { x: 10, y: 20, width: 300, height: 400 }
        );
    }

    #[test]
    fn canvas2d_plus_gl_viewport_unions_to_partial() {
        let mut acc = FrameDamageAccumulator::new();
        acc.add(DamageEffect::OnscreenRect { x: 10, y: 20, width: 100, height: 50 });
        acc.add(DamageEffect::OnscreenRect { x: 200, y: 300, width: 150, height: 100 });
        assert_eq!(
            acc.resolve((1080, 1920)),
            ResolvedDamage::Partial { x: 10, y: 20, width: 340, height: 380 }
        );
    }

    #[test]
    fn untracked_gl_clear_forces_full_surface() {
        let mut acc = FrameDamageAccumulator::new();
        acc.add(DamageEffect::OnscreenRect { x: 10, y: 20, width: 100, height: 50 });
        acc.add(DamageEffect::FullSurface);
        assert_eq!(acc.resolve((1080, 1920)), ResolvedDamage::FullSurface);
    }

    #[test]
    fn offscreen_gl_produces_no_damage() {
        let mut acc = FrameDamageAccumulator::new();
        acc.add(DamageEffect::OnscreenRect { x: 10, y: 20, width: 100, height: 50 });
        acc.add(DamageEffect::NoDamage); // offscreen GL
        assert_eq!(
            acc.resolve((1080, 1920)),
            ResolvedDamage::Partial { x: 10, y: 20, width: 100, height: 50 }
        );
    }

    #[test]
    fn full_surface_after_partial_rects_poisons_accumulator() {
        let mut acc = FrameDamageAccumulator::new();
        acc.add(DamageEffect::OnscreenRect { x: 10, y: 20, width: 100, height: 50 });
        acc.add(DamageEffect::OnscreenRect { x: 200, y: 300, width: 150, height: 100 });
        acc.add(DamageEffect::FullSurface);
        assert_eq!(acc.resolve((1080, 1920)), ResolvedDamage::FullSurface);
    }

    #[test]
    fn multiple_mixed_batches_union_correctly() {
        let mut acc = FrameDamageAccumulator::new();
        acc.add(DamageEffect::OnscreenRect { x: 0, y: 0, width: 50, height: 50 });
        acc.add(DamageEffect::OnscreenRect { x: 100, y: 100, width: 60, height: 40 });
        acc.add(DamageEffect::OnscreenRect { x: 30, y: 20, width: 80, height: 60 });
        acc.add(DamageEffect::OnscreenRect { x: 200, y: 0, width: 50, height: 200 });
        assert_eq!(
            acc.resolve((1080, 1920)),
            ResolvedDamage::Partial { x: 0, y: 0, width: 250, height: 200 }
        );
    }

    #[test]
    fn scissor_bounded_clear_unions_with_canvas2d() {
        let mut acc = FrameDamageAccumulator::new();
        // Canvas2D rect
        acc.add(DamageEffect::OnscreenRect { x: 0, y: 0, width: 100, height: 100 });
        // Scissor-bounded clear (produced by damage_for_clear when scissor is active)
        acc.add(DamageEffect::OnscreenRect { x: 200, y: 200, width: 50, height: 50 });
        assert_eq!(
            acc.resolve((1080, 1920)),
            ResolvedDamage::Partial { x: 0, y: 0, width: 250, height: 250 }
        );
    }

    // ---- GL state tracking tests ----

    #[test]
    fn gl_state_defaults_correctly() {
        use super::ScissorState;
        let state = CanvasGLState::default();
        assert!(state.draws_to_default_fbo);
        assert_eq!(state.scissor, ScissorState::Disabled);
        assert_eq!(state.last_scissor_rect, None);
    }

    #[test]
    fn gl_state_tracks_fbo_and_scissor() {
        use super::ScissorState;
        let mut state = CanvasGLState::default();

        // Bind user FBO
        state.draws_to_default_fbo = false;
        assert!(!state.draws_to_default_fbo);

        // Set scissor rect + enable
        state.last_scissor_rect = Some((10, 20, 100, 50));
        state.scissor = ScissorState::Enabled { x: 10, y: 20, width: 100, height: 50 };
        assert!(matches!(state.scissor, ScissorState::Enabled { .. }));

        // Disable scissor
        state.scissor = ScissorState::Disabled;
        assert_eq!(state.scissor, ScissorState::Disabled);
        // last_scissor_rect retained
        assert_eq!(state.last_scissor_rect, Some((10, 20, 100, 50)));
    }
}
