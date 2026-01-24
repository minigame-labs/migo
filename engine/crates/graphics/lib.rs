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

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum BoundContext {
    Resource,
    Canvas(shared::protocol::render_cmd::CanvasId),
}

/// Convert a platform-agnostic Surface into the onscreen "window" handle required by the current backend.
/// On Android/EGL, this is an ANativeWindow* expressed as usize.
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
