//! # Cross-Platform Render Backend Abstraction
//!
//! This module provides platform-agnostic rendering traits that enable
//! the engine to run on Android, iOS, Windows, macOS, and Linux.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                        RenderBackend Trait                          │
//! │                                                                     │
//! │   initialize() → create_surface() → make_current() → swap_buffers()│
//! └─────────────────────────────────────────────────────────────────────┘
//!                              │
//!          ┌──────────────────┼──────────────────┐
//!          ▼                  ▼                  ▼
//!    ┌───────────┐     ┌───────────┐     ┌───────────┐
//!    │  EGL/GL   │     │   Metal   │     │  D3D11    │
//!    │ (Android/ │     │  (iOS/    │     │ (Windows) │
//!    │  Linux)   │     │  macOS)   │     │           │
//!    └───────────┘     └───────────┘     └───────────┘
//! ```

mod traits;
mod egl;

pub use traits::*;
pub use egl::EglBackend;

/// Supported rendering API backends
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendType {
    /// OpenGL ES via EGL (Android, Linux)
    OpenGLES,
    /// OpenGL via EGL (Desktop Linux, Windows via ANGLE)
    OpenGL,
    /// Metal (iOS, macOS) - Future
    Metal,
    /// Direct3D 11 (Windows) - Future
    Direct3D11,
    /// Vulkan (Android, Windows, Linux) - Future
    Vulkan,
}

impl BackendType {
    /// Get the default backend for the current platform
    pub fn default_for_platform() -> Self {
        #[cfg(target_os = "android")]
        { BackendType::OpenGLES }
        
        #[cfg(target_os = "ios")]
        { BackendType::Metal }
        
        #[cfg(target_os = "macos")]
        { BackendType::Metal }
        
        #[cfg(target_os = "windows")]
        { BackendType::Direct3D11 }
        
        #[cfg(target_os = "linux")]
        { BackendType::OpenGL }
        
        #[cfg(not(any(
            target_os = "android",
            target_os = "ios",
            target_os = "macos",
            target_os = "windows",
            target_os = "linux"
        )))]
        { BackendType::OpenGL }
    }
}

/// Texture format for surface and framebuffers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    RGBA8,
    BGRA8,
    RGB565,
    RGBA16F,
}

/// Surface configuration for creating render surfaces
#[derive(Debug, Clone)]
pub struct SurfaceConfig {
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
    pub depth_bits: u8,
    pub stencil_bits: u8,
    pub sample_count: u8,
    pub vsync: bool,
}

impl Default for SurfaceConfig {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            pixel_format: PixelFormat::RGBA8,
            depth_bits: 0,
            stencil_bits: 0,
            sample_count: 1,
            vsync: true,
        }
    }
}
