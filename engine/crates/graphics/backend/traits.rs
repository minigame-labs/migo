//! Core traits for cross-platform rendering backends

use shared::error::EngineResult;
use super::{BackendType, SurfaceConfig};

/// Unique identifier for a render surface/context
pub type SurfaceId = u32;

/// Native window handle abstraction
pub trait NativeWindow: Send + Sync {
    /// Get the raw pointer to the native window
    fn as_raw(&self) -> *mut std::ffi::c_void;
    
    /// Get the current width of the window
    fn width(&self) -> u32;
    
    /// Get the current height of the window  
    fn height(&self) -> u32;
    
    /// Get the device pixel ratio (for HiDPI support)
    fn scale_factor(&self) -> f32 { 1.0 }
}

/// Core rendering backend trait
///
/// This trait abstracts the platform-specific graphics API initialization
/// and surface management. Implementations handle the low-level details
/// of OpenGL ES/EGL, Metal, Direct3D, etc.
///
/// Note: The backend is NOT required to be Send+Sync as it will be used
/// exclusively from the render thread.
pub trait RenderBackend {
    /// Get the backend type
    fn backend_type(&self) -> BackendType;
    
    /// Create an onscreen surface from a native window
    ///
    /// # Arguments
    /// * `window` - Platform-native window handle
    /// * `config` - Surface configuration (format, depth, etc.)
    ///
    /// # Returns
    /// A unique surface ID that can be used for subsequent operations
    fn create_onscreen_surface(
        &mut self,
        window: usize,
        config: &SurfaceConfig,
    ) -> EngineResult<SurfaceId>;
    
    /// Create an offscreen surface (for render-to-texture)
    fn create_offscreen_surface(
        &mut self,
        width: u32,
        height: u32,
        config: &SurfaceConfig,
    ) -> EngineResult<SurfaceId>;
    
    /// Destroy a surface and free its resources
    fn destroy_surface(&mut self, surface_id: SurfaceId) -> EngineResult<()>;
    
    /// Resize a surface (typically after window resize)
    fn resize_surface(
        &mut self,
        surface_id: SurfaceId,
        width: u32,
        height: u32,
    ) -> EngineResult<()>;
    
    /// Make a surface current for rendering
    ///
    /// All subsequent GL/rendering calls will target this surface
    fn make_current(&mut self, surface_id: SurfaceId) -> EngineResult<()>;
    
    /// Make no surface current (unbind)
    fn make_none_current(&mut self) -> EngineResult<()>;
    
    /// Swap buffers (present the frame)
    ///
    /// # Arguments
    /// * `surface_id` - The surface to present
    /// * `wait_vsync` - Whether to wait for vertical sync
    fn swap_buffers(&mut self, surface_id: SurfaceId, wait_vsync: bool) -> EngineResult<()>;
    
    /// Get the GL function loader for this backend
    ///
    /// Returns a function that can load GL function pointers by name
    fn get_proc_address(&self, name: &str) -> *const std::ffi::c_void;
    
    /// Query surface dimensions
    fn get_surface_size(&self, surface_id: SurfaceId) -> EngineResult<(u32, u32)>;
    
    /// Check if the backend supports a feature
    fn supports_feature(&self, feature: BackendFeature) -> bool;
}

/// Backend feature capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendFeature {
    /// Asynchronous buffer uploads (PBO in GL)
    AsyncBufferUpload,
    /// Multiple render targets (MRT)
    MultipleRenderTargets,
    /// Compute shaders
    ComputeShaders,
    /// MSAA antialiasing
    MSAA,
    /// HDR rendering
    HDR,
    /// Partial surface updates (for dirty rect optimization)
    PartialPresent,
}

/// Frame timing information for profiling and adaptive frame rate
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameTiming {
    /// Time spent on CPU (command recording)
    pub cpu_time_us: u64,
    /// Time spent on GPU (estimated)
    pub gpu_time_us: u64,
    /// Time waiting for vsync
    pub vsync_wait_us: u64,
    /// Total frame time
    pub total_time_us: u64,
}

/// VSync mode options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VSyncMode {
    /// No vsync, render as fast as possible
    Off,
    /// Standard vsync, wait for next refresh
    #[default]
    On,
    /// Adaptive vsync (tear if late, wait if early)
    Adaptive,
    /// Triple buffering with vsync
    TripleBuffer,
}
