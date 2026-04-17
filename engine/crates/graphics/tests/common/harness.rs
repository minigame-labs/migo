//! Surface construction helpers for drawing tests.
//!
//! Provides two paths:
//!
//!   * [`with_raster_surface`]: CPU-backed `SkSurface` — no GL context
//!     required.  Used by Phase 4 pixel golden tests.  Matches GPU output
//!     for every Canvas2D API since Skia's raster backend is the spec'd
//!     reference implementation for its 2D API.
//!   * [`with_gl_surface`]: GPU-backed `SkSurface` on an offscreen EGL
//!     pbuffer (Phase 6+).  Returns `None` when running on a host that
//!     lacks a working GLES 3.0 driver (e.g. CI sandbox with no Mesa);
//!     callers should treat that case as "test skipped".
//!
//! Both entry points take a closure to ensure Skia resources are dropped
//! before the EGL context is torn down.

use skia_safe::{surfaces, ColorType, ISize, ImageInfo, Surface};

/// Run `f` with a fresh CPU-backed Skia surface of size `(w, h)` in
/// un-premultiplied RGBA8888 colour space.
///
/// The surface is cleared to transparent black before the closure runs,
/// matching Canvas 2D spec §2.4 "Initial state".
pub fn with_raster_surface<T>(w: i32, h: i32, f: impl FnOnce(&mut Surface) -> T) -> T {
    let info = ImageInfo::new(
        ISize::new(w, h),
        ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        None,
    );
    let mut surface = surfaces::raster(&info, None, None)
        .expect("skia_safe::surfaces::raster returned None for valid ImageInfo");

    // The raster surface allocates zeroed memory; explicitly clear to make
    // the contract with tests obvious.
    surface.canvas().clear(skia_safe::Color::TRANSPARENT);
    f(&mut surface)
}

/// Read back the full surface contents as a tightly-packed RGBA8888 buffer
/// in `unpremul` colour space (matching the canvas `getImageData` API).
///
/// Returns a `Vec` of size `w * h * 4`.
pub fn read_pixels_rgba8(surface: &mut Surface) -> Vec<u8> {
    let image_info = surface.image_info();
    let w = image_info.width();
    let h = image_info.height();
    let row_bytes = (w * 4) as usize;
    let mut out = vec![0u8; row_bytes * h as usize];
    let info = ImageInfo::new(
        ISize::new(w, h),
        ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        None,
    );
    let read_ok = surface.read_pixels(&info, &mut out, row_bytes, (0, 0));
    assert!(read_ok, "SkSurface::read_pixels failed");
    out
}

/// Offscreen GL pbuffer + `GrDirectContext` setup; returns `None` on hosts
/// without working EGL/GLES3.  Not used by Phase 4 goldens; reserved for
/// Phase 6+ GL-specific coverage so that sandbox CI machines don't block
/// progress on Phase 4.
#[cfg(any())]
pub fn with_gl_surface<T>(
    _w: i32,
    _h: i32,
    _f: impl FnOnce(&mut Surface) -> T,
) -> Option<T> {
    // Deliberately unimplemented until Phase 6.  Keeping the symbol in the
    // module acts as a reminder that we will need it.
    None
}
