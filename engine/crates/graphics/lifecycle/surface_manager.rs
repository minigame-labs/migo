//! Surface management for lifecycle transitions
//!
//! Handles EGL surface creation, destruction, and recreation during
//! app lifecycle changes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tracing::{info, warn, error};

/// Surface state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceState {
    /// No surface
    None,
    /// Surface is being created
    Creating,
    /// Surface is valid and ready
    Valid,
    /// Surface is being destroyed
    Destroying,
    /// Surface was lost (context lost)
    Lost,
}

/// Information about the current surface
#[derive(Debug, Clone)]
pub struct SurfaceInfo {
    /// Native window handle
    pub native_window: usize,
    /// Surface width in pixels
    pub width: u32,
    /// Surface height in pixels
    pub height: u32,
    /// Device pixel ratio
    pub dpi: f32,
    /// Refresh rate (Hz)
    pub refresh_rate: u32,
}

impl Default for SurfaceInfo {
    fn default() -> Self {
        Self {
            native_window: 0,
            width: 0,
            height: 0,
            dpi: 1.0,
            refresh_rate: 60,
        }
    }
}

/// Manages surface lifecycle
pub struct SurfaceManager {
    /// Current state
    state: SurfaceState,
    /// Current surface info
    info: SurfaceInfo,
    /// Pending surface info (for recreation)
    pending_info: Option<SurfaceInfo>,
    /// Whether context was lost
    context_lost: AtomicBool,
    /// Frame count since surface creation
    frame_count: u64,
}

impl SurfaceManager {
    /// Create a new surface manager
    pub fn new() -> Self {
        Self {
            state: SurfaceState::None,
            info: SurfaceInfo::default(),
            pending_info: None,
            context_lost: AtomicBool::new(false),
            frame_count: 0,
        }
    }
    
    /// Get current state
    pub fn state(&self) -> SurfaceState {
        self.state
    }
    
    /// Get current surface info
    pub fn info(&self) -> &SurfaceInfo {
        &self.info
    }
    
    /// Check if surface is valid for rendering
    pub fn is_valid(&self) -> bool {
        self.state == SurfaceState::Valid && !self.context_lost.load(Ordering::Acquire)
    }
    
    /// Called when surface is created
    pub fn on_surface_created(&mut self, native_window: usize, width: u32, height: u32, dpi: f32) {
        info!("Surface created: {}x{} @ {}x DPI, window={:#x}", 
              width, height, dpi, native_window);
        
        self.info = SurfaceInfo {
            native_window,
            width,
            height,
            dpi,
            refresh_rate: 60, // Will be updated if available
        };
        
        self.state = SurfaceState::Valid;
        self.context_lost.store(false, Ordering::Release);
        self.frame_count = 0;
    }
    
    /// Called when surface size changes
    pub fn on_surface_changed(&mut self, width: u32, height: u32) {
        if self.info.width == width && self.info.height == height {
            return;
        }
        
        info!("Surface changed: {}x{} -> {}x{}", 
              self.info.width, self.info.height, width, height);
        
        self.info.width = width;
        self.info.height = height;
    }
    
    /// Called when surface is destroyed
    pub fn on_surface_destroyed(&mut self) {
        info!("Surface destroyed");
        
        // Save current info for potential recreation
        if self.state == SurfaceState::Valid {
            self.pending_info = Some(self.info.clone());
        }
        
        self.state = SurfaceState::None;
        self.info.native_window = 0;
    }
    
    /// Called when GL context is lost
    pub fn on_context_lost(&mut self) {
        warn!("GL context lost!");
        self.context_lost.store(true, Ordering::Release);
        self.state = SurfaceState::Lost;
    }
    
    /// Check if we have pending surface to recreate
    pub fn has_pending_surface(&self) -> bool {
        self.pending_info.is_some()
    }
    
    /// Get pending surface info
    pub fn take_pending_surface(&mut self) -> Option<SurfaceInfo> {
        self.pending_info.take()
    }
    
    /// Prepare for surface recreation
    pub fn prepare_recreation(&mut self) -> Option<SurfaceInfo> {
        if self.state == SurfaceState::None && self.pending_info.is_some() {
            self.state = SurfaceState::Creating;
            return self.pending_info.clone();
        }
        None
    }
    
    /// Complete surface recreation
    pub fn complete_recreation(&mut self, native_window: usize) {
        if let Some(info) = &self.pending_info {
            self.info = SurfaceInfo {
                native_window,
                width: info.width,
                height: info.height,
                dpi: info.dpi,
                refresh_rate: info.refresh_rate,
            };
            self.state = SurfaceState::Valid;
            self.context_lost.store(false, Ordering::Release);
            self.pending_info = None;
            info!("Surface recreation complete");
        }
    }
    
    /// Increment frame count
    pub fn on_frame(&mut self) {
        self.frame_count += 1;
    }
    
    /// Get frame count since surface creation
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }
    
    /// Get logical dimensions (CSS pixels)
    pub fn logical_size(&self) -> (u32, u32) {
        let dpi = self.info.dpi.max(1.0);
        (
            (self.info.width as f32 / dpi).round() as u32,
            (self.info.height as f32 / dpi).round() as u32,
        )
    }
    
    /// Get physical dimensions
    pub fn physical_size(&self) -> (u32, u32) {
        (self.info.width, self.info.height)
    }
}

impl Default for SurfaceManager {
    fn default() -> Self {
        Self::new()
    }
}
