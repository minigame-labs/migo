//! # Graphics Rendering Module
//!
//! This crate provides 2D and WebGL rendering capabilities for the Migo engine,
//! implementing Canvas 2D and WebGL 1.0 APIs.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                       Render Thread                              │
//! │                                                                  │
//! │  ┌───────────────────────────────────────────────────────────┐  │
//! │  │                    CanvasManager                          │  │
//! │  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐      │  │
//! │  │  │   Canvas 1  │  │   Canvas 2  │  │   Canvas N  │  ... │  │
//! │  │  │  (onscreen) │  │ (offscreen) │  │ (offscreen) │      │  │
//! │  │  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘      │  │
//! │  │         │                │                │              │  │
//! │  │  ┌──────▼──────┐  ┌──────▼──────┐  ┌──────▼──────┐      │  │
//! │  │  │ EGL Context │  │ EGL Context │  │ EGL Context │      │  │
//! │  │  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘      │  │
//! │  │         │                │                │              │  │
//! │  │  ┌──────▼────────────────▼────────────────▼──────┐      │  │
//! │  │  │                Shared Resource Context         │      │  │
//! │  │  │           (textures, shaders, etc.)           │      │  │
//! │  │  └───────────────────────────────────────────────┘      │  │
//! │  └───────────────────────────────────────────────────────────┘  │
//! │                                                                  │
//! │  ┌─────────────────┐  ┌─────────────────┐                       │
//! │  │   Renderer2D    │  │   RendererGL    │                       │
//! │  │   (Canvas2D)    │  │   (WebGL 1.0)   │                       │
//! │  │                 │  │                 │                       │
//! │  │  ┌───────────┐  │  │  ┌───────────┐  │                       │
//! │  │  │  FemtoVG  │  │  │  │   glow    │  │                       │
//! │  │  │           │  │  │  │ (OpenGL)  │  │                       │
//! │  │  └───────────┘  │  │  └───────────┘  │                       │
//! │  └─────────────────┘  └─────────────────┘                       │
//! └─────────────────────────────────────────────────────────────────┘
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
//!   - Dirty flag tracking for minimal redraws
//!   - Batched command processing
//!   - Frame rate control (default 60 FPS)
//!
//! ## Platform Support
//!
//! - **Android**: OpenGL ES 2.0/3.0 via EGL
//! - **Desktop**: OpenGL via EGL (for testing)
//!
//! ## Module Structure
//!
//! - [`render_thread`]: Main render thread and command dispatcher
//! - [`canvas`]: Canvas and EGL context management
//! - [`renderer2d`]: Canvas 2D rendering via FemtoVG
//! - [`renderergl`]: WebGL command handler

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
