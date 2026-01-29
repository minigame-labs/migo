//! # Graphics Rendering Module
//!
//! This crate provides 2D and WebGL rendering capabilities for the Migo engine,
//! implementing Canvas 2D and WebGL 1.0 APIs.
//!
//! ## Architecture (V2)
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                           JS Thread                                  │
//! │                                                                      │
//! │   RAF → CommandEncoder.fill_rect() → .draw_image() → .finish()      │
//! │                                            │                         │
//! │                                            ▼                         │
//! │                                    CommandBuffer                     │
//! │                                            │                         │
//! └────────────────────────────────────────────┼─────────────────────────┘
//!                                              │
//!                            FrameSubmitter.submit(buffer)
//!                                              │
//!                                              ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                         Render Thread                                │
//! │                                                                      │
//! │   ┌──────────────┐    ┌──────────────┐    ┌──────────────┐          │
//! │   │    Frame     │    │   Command    │    │   Backend    │          │
//! │   │  Scheduler   │───▶│   Executor   │───▶│ (EGL/Metal/ │          │
//! │   │              │    │              │    │   D3D11)    │          │
//! │   └──────────────┘    └──────────────┘    └──────────────┘          │
//! │                                                                      │
//! │   Features:                                                          │
//! │   - Command batching (1 message per frame)                          │
//! │   - On-demand rendering (zero GPU when idle)                        │
//! │   - VSync-aware frame pacing                                        │
//! │   - Cross-platform backend abstraction                              │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Features
//!
//! - **Canvas 2D API**: Full HTML5 Canvas 2D context implementation
//!   - Path drawing (moveTo, lineTo, arc, bezierCurveTo, etc.)
//!   - Fill and stroke styles (solid colors, gradients, patterns)
//!   - Text rendering (fillText, strokeText, measureText)
//!   - Image drawing (drawImage with scaling and clipping)
//!   - Transformations (translate, rotate, scale, setTransform)
//!   - Compositing and blending modes
//!
//! - **WebGL 1.0 API**: OpenGL ES 2.0 compatible WebGL implementation
//!   - Shaders (vertex, fragment) and programs
//!   - Buffers (vertex, index)
//!   - Textures (2D, with mipmaps)
//!   - Framebuffers and renderbuffers
//!   - Blend, depth, and stencil operations
//!
//! - **Multi-Canvas Support**: Multiple offscreen canvases with shared resources
//!
//! - **Efficient Rendering**:
//!   - Command batching (reduced IPC overhead)
//!   - Dirty region tracking
//!   - On-demand rendering mode
//!   - Frame pacing with VSync
//!
//! ## Platform Support
//!
//! - **Android**: OpenGL ES 2.0/3.0 via EGL
//! - **iOS**: Metal (planned)
//! - **Windows**: Direct3D 11 or OpenGL via ANGLE (planned)
//! - **macOS**: Metal (planned)
//! - **Linux**: OpenGL via EGL
//!
//! ## Module Structure
//!
//! - [`backend`]: Cross-platform rendering backend traits and implementations
//! - [`command_buffer`]: Command batching system
//! - [`scheduler`]: Frame scheduling and timing
//! - [`render_thread`]: Legacy render thread (V1)
//! - [`render_thread_v2`]: New optimized render thread
//! - [`canvas`]: Canvas and EGL context management
//! - [`renderer2d`]: Canvas 2D rendering via FemtoVG
//! - [`renderergl`]: WebGL command handler

// Core modules
mod canvas;
mod render_thread;
mod renderer2d;
mod renderergl;

// V2 architecture modules (command batching, frame scheduling)
pub mod command_buffer;
pub mod scheduler;

// Performance optimization modules
pub mod batching;
pub mod pool;
pub mod dirty_region;
pub mod lifecycle;

// Backend abstraction module (for future cross-platform support)
// Note: EglBackend implementation requires careful thread-safety handling
// and is currently not enabled by default.
#[cfg(feature = "backend_v2")]
pub mod backend;

// V2 render thread (requires backend_v2 feature)
#[cfg(feature = "backend_v2")]
mod render_thread_v2;

pub(crate) use canvas::*;
pub use render_thread::*;

pub(crate) use renderer2d::*;
pub(crate) use renderergl::*;

// Re-export key types from new modules (V2 command batching)
pub use command_buffer::{CommandBuffer, CommandEncoder, Canvas2DCommand};
pub use scheduler::{FrameConfig, FrameScheduler, RenderMode, FrameSubmitter};

// Re-export optimization types
pub use batching::{DrawBatcher, DrawOp, BatchStats, TextureAtlas, AtlasManager, GLState};
pub use pool::{BufferPool, PathPool, ObjectPool, PoolStats};
pub use dirty_region::{DirtyRect, DirtyRegionTracker, DirtyStats};
pub use lifecycle::{AppLifecycleState, AppStateManager, LifecycleConfig, SharedLifecycleState};

// V2 render thread (when backend_v2 feature is enabled)
#[cfg(feature = "backend_v2")]
pub use render_thread_v2::RenderThreadV2;

use raw_window_handle::RawWindowHandle;
use shared::error::{EngineResult, ErrorCode};
use shared::surface::Surface;

/// Tracks which EGL context is currently bound.
///
/// Used to avoid redundant `eglMakeCurrent` calls and ensure proper
/// context switching when rendering to multiple canvases.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum BoundContext {
    /// The shared resource context is bound (for loading textures, compiling shaders).
    Resource,
    /// A specific canvas context is bound for rendering.
    Canvas(shared::protocol::render_cmd::CanvasId),
}

/// Convert a platform-agnostic Surface into the onscreen "window" handle.
///
/// This extracts the native window handle from the platform abstraction,
/// which is required by EGL to create a window surface.
///
/// # Platform Details
///
/// - **Android**: Returns `ANativeWindow*` as `usize`
/// - **Other platforms**: Not yet supported
///
/// # Errors
///
/// Returns `ErrorCode::Unsupported` if the window handle type is not
/// supported by the current graphics backend.
pub(crate) fn onscreen_window_from_surface(surface: &dyn Surface) -> EngineResult<usize> {
    match surface.raw_window_handle() {
        RawWindowHandle::AndroidNdk(h) => Ok(h.a_native_window.as_ptr() as usize),
        other => {
            shared::bail!(
                ErrorCode::Unsupported,
                "unsupported RawWindowHandle for current backend",
                format!("{:?}", other)
            );
        }
    }
}
