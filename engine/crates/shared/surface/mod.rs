//! # Surface Abstraction Module
//!
//! Provides platform-agnostic abstractions for rendering surfaces and window geometry.
//!
//! ## Overview
//!
//! A "surface" represents a drawable area where graphics can be rendered.
//! On Android, this wraps an `ANativeWindow`; on desktop platforms, it could
//! wrap a window handle from winit or similar libraries.
//!
//! ## Key Types
//!
//! - [`Surface`]: Trait for platform-specific surface implementations
//! - [`SurfaceRef`]: Thread-safe reference to a surface (`Arc<dyn Surface>`)
//! - [`WindowInfo`]: Logical window dimensions and pixel ratio
//! - [`SafeArea`]: Device safe area insets (notch, navigation bar, etc.)
//!
//! ## Platform Integration
//!
//! Each platform crate provides its own `Surface` implementation:
//!
//! ```rust,ignore
//! // Android implementation (in platform crate)
//! pub struct AndroidSurface {
//!     native_window: NonNull<ANativeWindow>,
//!     width: u32,
//!     height: u32,
//! }
//!
//! impl Surface for AndroidSurface {
//!     fn size(&self) -> (u32, u32) {
//!         (self.width, self.height)
//!     }
//!
//!     fn raw_window_handle(&self) -> RawWindowHandle {
//!         RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(
//!             self.native_window
//!         ))
//!     }
//!     // ...
//! }
//! ```

use std::sync::Arc;

mod geometry;

pub use geometry::{SafeArea, WindowInfo};

/// A platform window/surface abstraction used by the renderer.
///
/// This trait provides the minimal interface needed to create an EGL/OpenGL
/// rendering context and draw to the screen.
///
/// # Thread Safety
///
/// Implementations must be `Send + Sync` to allow passing surfaces between
/// threads (e.g., from the platform layer to the render thread).
///
/// # Platform Implementation Notes
///
/// - **Android**: Wraps `ANativeWindow*` from the NDK
/// - **Desktop (future)**: Could wrap winit `Window` or raw X11/Wayland handles
pub trait Surface: std::fmt::Debug + Send + Sync {
    /// Returns the physical size of the surface in pixels.
    ///
    /// This is the actual framebuffer size, not the logical (CSS pixel) size.
    /// Use this for `glViewport` and texture allocation.
    ///
    /// # Returns
    ///
    /// Tuple of `(width, height)` in physical pixels.
    fn size(&self) -> (u32, u32);

    /// Returns the raw window handle for native graphics API integration.
    ///
    /// This handle is used by EGL/OpenGL to create a rendering surface.
    /// The specific handle type depends on the platform:
    ///
    /// - `AndroidNdk`: Contains `ANativeWindow*`
    /// - `Xlib`/`Wayland`: Contains X11/Wayland window handles
    /// - `Win32`: Contains HWND
    fn raw_window_handle(&self) -> raw_window_handle::RawWindowHandle;

    /// Returns the raw display handle for the platform's display server.
    ///
    /// Required by EGL to connect to the display system:
    ///
    /// - `AndroidNdk`: No display handle needed (returns Android variant)
    /// - `Xlib`: Contains X11 `Display*`
    /// - `Wayland`: Contains `wl_display*`
    fn raw_display_handle(&self) -> raw_window_handle::RawDisplayHandle;

    /// Monotonic destroy-epoch captured when this surface was handed off from
    /// the platform layer. The render thread compares it against the live
    /// per-host destroy counter each frame: if a `surfaceDestroyed` has occurred
    /// since (live counter advanced past this value), the surface is stale and
    /// must not be presented to. Defaults to 0 for platforms without surface
    /// teardown semantics.
    fn surface_epoch(&self) -> u64 {
        0
    }
}

/// Thread-safe reference to a surface.
///
/// This type alias provides a convenient way to pass surfaces between
/// threads without ownership concerns. The underlying surface is
/// reference-counted and can be safely cloned.
///
/// # Example
///
/// ```rust,ignore
/// use shared::surface::SurfaceRef;
///
/// fn spawn_render_thread(surface: SurfaceRef) {
///     std::thread::spawn(move || {
///         let (width, height) = surface.size();
///         // Create EGL context with surface...
///     });
/// }
/// ```
pub type SurfaceRef = Arc<dyn Surface + Send + Sync + 'static>;
