extern crate khronos_egl as egl;

use crate::{BoundContext, Canvas2DContext};
use egl::EGL1_4;
use femtovg::ImageId;
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
    ee, BufferMeta, CanvasGLState, CanvasInfo, FramebufferMeta, ProgramMeta, RenderbufferMeta,
    ShaderMeta, TextureMeta,
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
}

impl CanvasManager {
    pub(crate) fn new_with_resource(egl_lib_path: &str, dpi: f32) -> EngineResult<Self> {
        let init = egl_ops::init_egl(egl_lib_path)?;
        let egl = init.egl;
        let display = init.display;
        let config = init.config;

        // Create resource context + pbuffer.
        let (resource_ctx, resource_surf) =
            egl_ops::create_pbuffer_context(&egl, display, config, None, 16, 16)?;
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
            egl_ops::create_pbuffer_context(&self.egl, self.display, self.config, share, w, h)?;

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
            },
        );

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

        self.egl.bind_api(egl::OPENGL_ES_API).map_err(|e| {
            ee(
                ErrorCode::RenderBackendError,
                format!("eglBindAPI failed: {e:?}"),
            )
        })?;

        let surf = egl_ops::create_window_surface(&self.egl, self.display, self.config, window)?;

        let ctx_attribs = [egl::CONTEXT_CLIENT_VERSION as i32, 2, egl::NONE as i32];
        let ctx = self
            .egl
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
            })?;

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
            },
        );

        if let Some(ctx2d) = self.contexts_2d.get_mut(&id) {
            // dpi = 1.0: Canvas2D coordinates are in buffer pixels (no DPR scaling)
            ctx2d.canvas.set_size(physical_w, physical_h, 1.0);
        }

        // Re-initialize the femtovg 2D context if one existed before the old
        // onscreen was destroyed. This happens on Android resume where the
        // surface is a different native window but the game's JS code still
        // expects canvas_id=1's 2D context to work.
        if had_2d_context && !self.contexts_2d.contains_key(&id) {
            context_2d_impl::init_femtovg_for_canvas(self, id)?;
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
            let _ = self.egl.destroy_context(self.display, entry.ctx.ctx);

            self.bound = BoundContext::Resource;
            self.last_swap_interval = -1;
        }
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
    pub(crate) fn is_onscreen_bound(&self) -> bool {
        self.bound == BoundContext::Canvas(CanvasId::from(1u32))
    }

    /// Returns the DrawingBuffer FBO for the given canvas, or None (= real FBO 0)
    /// if the canvas has no DrawingBuffer (offscreen canvases).
    pub(crate) fn get_drawing_buffer_fbo(
        &self,
        canvas_id: CanvasId,
    ) -> Option<glow::NativeFramebuffer> {
        self.canvases
            .get(&canvas_id)
            .and_then(|entry| entry.drawing_buffer.as_ref())
            .map(|db| db.fbo)
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

            if let Some(ctx2d) = self.contexts_2d.get_mut(&id) {
                // dpi = 1.0: Canvas2D coordinates are in buffer pixels (no DPR scaling)
                ctx2d.canvas.set_size(new_w, new_h, 1.0);
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

        if let Some(ctx2d) = self.contexts_2d.get_mut(&id) {
            // dpi = 1.0: Canvas2D coordinates are in buffer pixels (no DPR scaling)
            ctx2d.canvas.set_size(actual_w, actual_h, 1.0);
        }

        if !was_current {
            self.restore_bound(saved_bound)?;
        }

        Ok(())
    }

    pub(crate) fn swap_buffers_no_restore(
        &mut self,
        id: CanvasId,
        wait_for_vsync: bool,
    ) -> EngineResult<()> {
        self.make_current_needed(id)?;
        let entry = self
            .canvases
            .get(&id)
            .ok_or_else(|| ee(ErrorCode::NotFound, format!("canvas not found: {id:?}")))?;

        // Blit DrawingBuffer to the real window surface before swap.
        if let Some(ref db) = entry.drawing_buffer {
            drawing_buffer::blit_to_surface(
                &self.gl,
                db,
                entry.physical_width,
                entry.physical_height,
            );
        }

        // Only call eglSwapInterval when the value actually changes
        let interval = if wait_for_vsync { 1 } else { 0 };
        if interval != self.last_swap_interval {
            let _ = self.egl.swap_interval(self.display, interval);
            self.last_swap_interval = interval;
        }
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
        if let Some(ref db) = self
            .canvases
            .get(&id)
            .and_then(|e| e.drawing_buffer.as_ref())
        {
            unsafe {
                self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(db.fbo));
            }
        }

        Ok(())
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

    pub(crate) fn init_femtovg_for_canvas(&mut self, canvas_id: CanvasId) -> EngineResult<()> {
        context_2d_impl::init_femtovg_for_canvas(self, canvas_id)
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

    /// Iterate over all 2D contexts mutably (for font registration, etc.).
    pub(crate) fn contexts_2d_iter_mut(
        &mut self,
    ) -> impl Iterator<Item = (&CanvasId, &mut Canvas2DContext)> {
        self.contexts_2d.iter_mut()
    }

    pub(crate) fn mark_2d_dirty(&mut self, canvas_id: CanvasId) {
        self.dirty_2d.insert(canvas_id);
    }

    pub(crate) fn pending_dirty_2d_count(&self) -> usize {
        self.dirty_2d.len()
    }

    /// Save current GL state and set a safe baseline for Canvas2D / femtovg
    /// text atlas uploads.
    pub(crate) fn begin_canvas2d_gl_scope(&self) -> context_2d_impl::Canvas2DGlScopeGuard {
        context_2d_impl::begin_canvas2d_gl_scope(&self.gl)
    }

    pub(crate) fn flush_dirty_2d_contexts(&mut self) -> EngineResult<()> {
        context_2d_impl::flush_dirty_2d_contexts(self)
    }

    /// Read pixel data from the current framebuffer via glReadPixels.
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

    // ==================== Image Management ====================

    pub(crate) fn generate_img_id(&self) -> u32 {
        self.image_registry.generate_img_id()
    }

    pub(crate) fn load_shared_fv_image(
        &mut self,
        image_id: u32,
        image: NormalizedImage,
    ) -> EngineResult<(u32, u32)> {
        self.ensure_any_canvas_current()?;
        self.image_registry
            .load_shared_fv_image(&self.gl, image_id, image)
    }

    pub(crate) fn destroy_shared_fv_image(&mut self, image_id: u32) -> EngineResult<()> {
        self.ensure_any_canvas_current()?;

        // We need to split borrow here
        let gl = &self.gl;
        let display = self.display;
        let egl = &self.egl;
        let canvases = &self.canvases;
        let bound = &mut self.bound;

        self.image_registry.destroy_shared_fv_image(
            gl,
            image_id,
            |canvas_id| {
                // Inline make_current logic to avoid borrow issues
                if *bound == BoundContext::Canvas(canvas_id) {
                    return Ok(());
                }
                let entry = canvases.get(&canvas_id).ok_or_else(|| {
                    ee(
                        ErrorCode::NotFound,
                        format!("canvas not found: {canvas_id:?}"),
                    )
                })?;

                egl.make_current(
                    display,
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
                *bound = BoundContext::Canvas(canvas_id);
                Ok(())
            },
            &mut self.contexts_2d,
        )
    }

    pub(crate) fn get_shared_fv_image(
        &self,
        image_id: u32,
    ) -> Option<(u32, glow::NativeTexture, femtovg::ImageInfo)> {
        self.image_registry.get_shared_fv_image(image_id)
    }

    pub(crate) fn get_owned_fv_image(
        &self,
        image_id: u32,
        canvas_id: CanvasId,
    ) -> Option<(ImageId, glow::NativeTexture, femtovg::ImageInfo)> {
        self.image_registry.get_owned_fv_image(image_id, canvas_id)
    }

    /// Access to fv_images for external mutation
    pub(crate) fn fv_images_mut(
        &mut self,
    ) -> &mut HashMap<u32, HashMap<CanvasId, (ImageId, glow::NativeTexture, femtovg::ImageInfo)>>
    {
        &mut self.image_registry.fv_images
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
