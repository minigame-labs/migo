use std::sync::Arc;

mod geometry;

pub use geometry::{SafeArea, WindowInfo};

/// A platform window/surface abstraction used by the renderer.
pub trait Surface: std::fmt::Debug + Send + Sync {
    /// Physical size in pixels.
    fn size(&self) -> (u32, u32);

    fn raw_window_handle(&self) -> raw_window_handle::RawWindowHandle;

    fn raw_display_handle(&self) -> raw_window_handle::RawDisplayHandle;
}

/// Shared surface reference passed across threads/runtimes.
pub type SurfaceRef = Arc<dyn Surface + Send + Sync + 'static>;
