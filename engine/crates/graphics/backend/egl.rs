//! EGL-based OpenGL ES backend for Android and Linux

extern crate khronos_egl as egl;

use std::collections::HashMap;
use egl::EGL1_4;
use shared::error::{EngineResult, ErrorCode};
use tracing::info;

use super::{BackendFeature, BackendType, RenderBackend, SurfaceConfig, SurfaceId};

/// EGL context and surface handle
struct EglSurface {
    context: egl::Context,
    surface: egl::Surface,
    width: u32,
    height: u32,
    is_onscreen: bool,
}

/// EGL-based rendering backend
///
/// Supports OpenGL ES 2.0/3.0 on Android and desktop Linux.
/// Can also work on Windows via ANGLE.
pub struct EglBackend {
    egl: egl::DynamicInstance<EGL1_4>,
    display: egl::Display,
    config: egl::Config,
    
    /// Shared context for resource loading
    resource_context: egl::Context,
    resource_surface: egl::Surface,
    
    /// All active surfaces
    surfaces: HashMap<SurfaceId, EglSurface>,
    next_surface_id: SurfaceId,
    
    /// Currently bound surface
    current_surface: Option<SurfaceId>,
    
    /// Backend configuration
    use_gles3: bool,
}

impl EglBackend {
    /// Create a new EGL backend
    ///
    /// # Arguments
    /// * `egl_lib_path` - Path to the EGL library (e.g., "libEGL.so")
    pub fn new(egl_lib_path: &str) -> EngineResult<Self> {
        // Load EGL library
        let egl = unsafe {
            egl::DynamicInstance::<EGL1_4>::load_required_from(
                libloading::Library::new(egl_lib_path)
                    .map_err(|e| Self::ee(format!("failed to load EGL: {e}")))?,
            )
            .map_err(|e| Self::ee(format!("EGL1_4 not supported: {e}")))?
        };
        
        // Get display (unsafe because it accesses native display)
        let display = unsafe { egl.get_display(egl::DEFAULT_DISPLAY) }
            .ok_or_else(|| Self::ee("eglGetDisplay returned no display"))?;
        
        // Initialize EGL
        egl.initialize(display)
            .map_err(|e| Self::ee(format!("eglInitialize failed: {e:?}")))?;
        
        // Query EGL version (already initialized, just getting version)
        let version = egl.query_string(Some(display), egl::VERSION)
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        info!("EGL initialized: version {}", version);
        
        // Check GLES3 support before moving egl
        let use_gles3 = Self::check_gles3_support_static(&egl, display);
        
        // Choose config
        let config = Self::choose_config(&egl, display)?;
        
        // Create shared resource context
        let (resource_context, resource_surface) = 
            Self::create_resource_context(&egl, display, config)?;
        
        Ok(Self {
            egl,
            display,
            config,
            resource_context,
            resource_surface,
            surfaces: HashMap::with_capacity(4),
            next_surface_id: 1,
            current_surface: None,
            use_gles3,
        })
    }
    
    fn ee(msg: impl Into<String>) -> shared::error::EngineError {
        shared::error::EngineError::from_detail(ErrorCode::RenderBackendError, msg.into())
    }
    
    fn choose_config(
        egl: &egl::DynamicInstance<EGL1_4>,
        display: egl::Display,
    ) -> EngineResult<egl::Config> {
        let attribs = [
            egl::RED_SIZE, 8,
            egl::GREEN_SIZE, 8,
            egl::BLUE_SIZE, 8,
            egl::ALPHA_SIZE, 8,
            egl::DEPTH_SIZE, 0,
            egl::STENCIL_SIZE, 0,
            egl::SURFACE_TYPE, egl::PBUFFER_BIT | egl::WINDOW_BIT,
            egl::RENDERABLE_TYPE, egl::OPENGL_ES2_BIT,
            egl::NONE,
        ];
        
        egl.choose_first_config(display, &attribs)
            .map_err(|e| Self::ee(format!("eglChooseConfig failed: {e:?}")))?
            .ok_or_else(|| Self::ee("no suitable EGL config found"))
    }
    
    fn create_resource_context(
        egl: &egl::DynamicInstance<EGL1_4>,
        display: egl::Display,
        config: egl::Config,
    ) -> EngineResult<(egl::Context, egl::Surface)> {
        // Bind OpenGL ES API
        egl.bind_api(egl::OPENGL_ES_API)
            .map_err(|e| Self::ee(format!("eglBindAPI failed: {e:?}")))?;
        
        // Create pbuffer surface for resource context
        let pbuf_attribs = [
            egl::WIDTH as i32, 16,
            egl::HEIGHT as i32, 16,
            egl::NONE as i32,
        ];
        
        let surface = egl.create_pbuffer_surface(display, config, &pbuf_attribs)
            .map_err(|e| Self::ee(format!("create resource pbuffer failed: {e:?}")))?;
        
        // Create context
        let ctx_attribs = [
            egl::CONTEXT_CLIENT_VERSION as i32, 2,
            egl::NONE as i32,
        ];
        
        let context = egl.create_context(display, config, None, &ctx_attribs)
            .map_err(|e| Self::ee(format!("create resource context failed: {e:?}")))?;
        
        // Make current once to validate
        egl.make_current(display, Some(surface), Some(surface), Some(context))
            .map_err(|e| Self::ee(format!("make resource current failed: {e:?}")))?;
        
        Ok((context, surface))
    }
    
    fn check_gles3_support_static(
        egl: &egl::DynamicInstance<EGL1_4>,
        display: egl::Display,
    ) -> bool {
        // Check for GLES 3.0 support
        let extensions = egl.query_string(Some(display), egl::EXTENSIONS)
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        
        extensions.contains("EGL_KHR_create_context")
    }
    
    fn allocate_surface_id(&mut self) -> SurfaceId {
        let id = self.next_surface_id;
        self.next_surface_id += 1;
        id
    }
}

impl RenderBackend for EglBackend {
    fn backend_type(&self) -> BackendType {
        if self.use_gles3 {
            BackendType::OpenGLES
        } else {
            BackendType::OpenGLES
        }
    }
    
    fn create_onscreen_surface(
        &mut self,
        window: usize,
        _config: &SurfaceConfig,
    ) -> EngineResult<SurfaceId> {
        self.egl.bind_api(egl::OPENGL_ES_API)
            .map_err(|e| Self::ee(format!("eglBindAPI failed: {e:?}")))?;
        
        // Create window surface (unsafe because it uses native window handle)
        let native_window = window as egl::NativeWindowType;
        let surface = unsafe {
            self.egl
                .create_window_surface(self.display, self.config, native_window, None)
                .map_err(|e| Self::ee(format!("create window surface failed: {e:?}")))?
        };
        
        // Create context
        let ctx_attribs = [
            egl::CONTEXT_CLIENT_VERSION as i32, 2,
            egl::NONE as i32,
        ];
        
        let context = self.egl
            .create_context(self.display, self.config, Some(self.resource_context), &ctx_attribs)
            .map_err(|e| Self::ee(format!("create context failed: {e:?}")))?;
        
        // Query surface size
        let width = self.egl.query_surface(self.display, surface, egl::WIDTH)
            .unwrap_or(0) as u32;
        let height = self.egl.query_surface(self.display, surface, egl::HEIGHT)
            .unwrap_or(0) as u32;
        
        let id = self.allocate_surface_id();
        self.surfaces.insert(id, EglSurface {
            context,
            surface,
            width: width.max(1),
            height: height.max(1),
            is_onscreen: true,
        });
        
        info!("Created onscreen surface {}: {}x{}", id, width, height);
        
        Ok(id)
    }
    
    fn create_offscreen_surface(
        &mut self,
        width: u32,
        height: u32,
        _config: &SurfaceConfig,
    ) -> EngineResult<SurfaceId> {
        self.egl.bind_api(egl::OPENGL_ES_API)
            .map_err(|e| Self::ee(format!("eglBindAPI failed: {e:?}")))?;
        
        // Create pbuffer surface
        let pbuf_attribs = [
            egl::WIDTH as i32, width as i32,
            egl::HEIGHT as i32, height as i32,
            egl::NONE as i32,
        ];
        
        let surface = self.egl
            .create_pbuffer_surface(self.display, self.config, &pbuf_attribs)
            .map_err(|e| Self::ee(format!("create pbuffer failed: {e:?}")))?;
        
        // Create context (shared with resource context)
        let ctx_attribs = [
            egl::CONTEXT_CLIENT_VERSION as i32, 2,
            egl::NONE as i32,
        ];
        
        let context = self.egl
            .create_context(self.display, self.config, Some(self.resource_context), &ctx_attribs)
            .map_err(|e| Self::ee(format!("create context failed: {e:?}")))?;
        
        let id = self.allocate_surface_id();
        self.surfaces.insert(id, EglSurface {
            context,
            surface,
            width,
            height,
            is_onscreen: false,
        });
        
        info!("Created offscreen surface {}: {}x{}", id, width, height);
        
        Ok(id)
    }
    
    fn destroy_surface(&mut self, surface_id: SurfaceId) -> EngineResult<()> {
        if let Some(surf) = self.surfaces.remove(&surface_id) {
            // Unbind if current
            if self.current_surface == Some(surface_id) {
                let _ = self.egl.make_current(self.display, None, None, None);
                self.current_surface = None;
            }
            
            self.egl.destroy_surface(self.display, surf.surface).ok();
            self.egl.destroy_context(self.display, surf.context).ok();
            
            info!("Destroyed surface {}", surface_id);
        }
        
        Ok(())
    }
    
    fn resize_surface(
        &mut self,
        surface_id: SurfaceId,
        width: u32,
        height: u32,
    ) -> EngineResult<()> {
        let surf = self.surfaces.get_mut(&surface_id)
            .ok_or_else(|| Self::ee(format!("surface {} not found", surface_id)))?;
        
        // For onscreen surfaces, the size is determined by the window
        // For offscreen surfaces, we need to recreate the pbuffer
        if !surf.is_onscreen {
            // Destroy old surface
            if self.current_surface == Some(surface_id) {
                self.egl.make_current(self.display, None, None, None).ok();
                self.current_surface = None;
            }
            
            self.egl.destroy_surface(self.display, surf.surface).ok();
            
            // Create new pbuffer
            let pbuf_attribs = [
                egl::WIDTH as i32, width as i32,
                egl::HEIGHT as i32, height as i32,
                egl::NONE as i32,
            ];
            
            let new_surface = self.egl
                .create_pbuffer_surface(self.display, self.config, &pbuf_attribs)
                .map_err(|e| Self::ee(format!("recreate pbuffer failed: {e:?}")))?;
            
            surf.surface = new_surface;
        }
        
        surf.width = width;
        surf.height = height;
        
        Ok(())
    }
    
    fn make_current(&mut self, surface_id: SurfaceId) -> EngineResult<()> {
        if self.current_surface == Some(surface_id) {
            return Ok(());
        }
        
        let surf = self.surfaces.get(&surface_id)
            .ok_or_else(|| Self::ee(format!("surface {} not found", surface_id)))?;
        
        self.egl.make_current(
            self.display,
            Some(surf.surface),
            Some(surf.surface),
            Some(surf.context),
        ).map_err(|e| Self::ee(format!("make_current failed: {e:?}")))?;
        
        self.current_surface = Some(surface_id);
        
        Ok(())
    }
    
    fn make_none_current(&mut self) -> EngineResult<()> {
        self.egl.make_current(self.display, None, None, None)
            .map_err(|e| Self::ee(format!("make_none_current failed: {e:?}")))?;
        
        self.current_surface = None;
        
        Ok(())
    }
    
    fn swap_buffers(&mut self, surface_id: SurfaceId, wait_vsync: bool) -> EngineResult<()> {
        self.make_current(surface_id)?;
        
        let surf = self.surfaces.get(&surface_id)
            .ok_or_else(|| Self::ee(format!("surface {} not found", surface_id)))?;
        
        // Set swap interval
        let interval = if wait_vsync { 1 } else { 0 };
        let _ = self.egl.swap_interval(self.display, interval);
        
        self.egl.swap_buffers(self.display, surf.surface)
            .map_err(|e| Self::ee(format!("swap_buffers failed: {e:?}")))?;
        
        Ok(())
    }
    
    fn get_proc_address(&self, name: &str) -> *const std::ffi::c_void {
        self.egl
            .get_proc_address(name)
            .map(|f| f as *const std::ffi::c_void)
            .unwrap_or(std::ptr::null())
    }
    
    fn get_surface_size(&self, surface_id: SurfaceId) -> EngineResult<(u32, u32)> {
        let surf = self.surfaces.get(&surface_id)
            .ok_or_else(|| Self::ee(format!("surface {} not found", surface_id)))?;
        
        Ok((surf.width, surf.height))
    }
    
    fn supports_feature(&self, feature: BackendFeature) -> bool {
        match feature {
            BackendFeature::AsyncBufferUpload => true, // PBO support in GLES 3.0
            BackendFeature::MultipleRenderTargets => self.use_gles3,
            BackendFeature::ComputeShaders => false, // Requires GLES 3.1
            BackendFeature::MSAA => true,
            BackendFeature::HDR => false,
            BackendFeature::PartialPresent => false,
        }
    }
}

impl Drop for EglBackend {
    fn drop(&mut self) {
        // Destroy all surfaces
        let ids: Vec<SurfaceId> = self.surfaces.keys().copied().collect();
        for id in ids {
            let _ = self.destroy_surface(id);
        }
        
        // Destroy resource context
        let _ = self.egl.make_current(self.display, None, None, None);
        self.egl.destroy_surface(self.display, self.resource_surface).ok();
        self.egl.destroy_context(self.display, self.resource_context).ok();
        
        // Terminate display
        self.egl.terminate(self.display).ok();
        
        info!("EGL backend destroyed");
    }
}
