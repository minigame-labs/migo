//! # Graphics Rendering Module
//!
//! This crate provides 2D and WebGL rendering capabilities for the Migo engine,
//! implementing Canvas 2D and WebGL 1.0 APIs.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                           JS Thread                                  │
//! │                                                                      │
//! │   RAF → ctx.fillRect() → ctx.drawImage() → ... → RAF ends          │
//! │         FrameCommandCollector batches all commands per frame         │
//! │                                            │                         │
//! └────────────────────────────────────────────┼─────────────────────────┘
//!                                              │
//!                            Canvas2DBatch (single IPC per frame)
//!                                              │
//!                                              ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                         Render Thread                                │
//! │                                                                      │
//! │   RenderThread receives Canvas2DBatch / GL / Canvas commands         │
//! │   → renderer2d (femtovg) for 2D                                     │
//! │   → renderergl (glow) for WebGL                                     │
//! │   → EGL context management via canvas module                        │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Module Structure
//!
//! - [`render_thread`]: Render thread loop and command dispatch
//! - [`canvas`]: Canvas and EGL context management
//! - [`renderer2d`]: Canvas 2D rendering via femtovg
//! - [`renderergl`]: WebGL command handler via glow

mod canvas;
mod render_thread;
mod renderer2d;
mod renderergl;

pub(crate) use canvas::*;
pub use render_thread::*;

pub(crate) use renderer2d::*;
pub(crate) use renderergl::*;

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
