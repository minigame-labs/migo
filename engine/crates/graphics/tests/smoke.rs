//! Test-harness smoke tests.
//!
//! These tests exist to guarantee the harness itself works before any
//! Skia Canvas2D code relies on it.  Intentionally minimal: a clear + a
//! primitive shape.  Failures here indicate a harness bug, not a
//! rendering regression.
//!
//! Run with `cargo test -p graphics --test smoke`.

#[path = "common/mod.rs"]
mod common;

use common::golden::{GoldenCfg, assert_matches_golden};
use common::harness::{read_pixels_rgba8, with_raster_surface};
use skia_safe::{Color, Color4f, Paint, Rect};

/// Clearing the surface to solid red must produce `(255, 0, 0, 255)` at
/// every pixel.  Verified by direct pixel-read *and* by round-tripping
/// through a golden PNG — catches PNG encode/decode bugs in the harness.
#[test]
fn raster_surface_clear_to_red() {
    let (w, h) = (32, 32);
    let buf = with_raster_surface(w, h, |surface| {
        surface.canvas().clear(Color::RED);
        read_pixels_rgba8(surface)
    });

    assert_eq!(buf.len(), (w * h * 4) as usize);
    for chunk in buf.chunks_exact(4) {
        assert_eq!(chunk, &[255, 0, 0, 255]);
    }

    assert_matches_golden(
        "smoke_clear_red",
        w as u32,
        h as u32,
        &buf,
        GoldenCfg::default(),
    );
}

/// Draw a filled sub-rectangle in lime over a cleared white background.
/// The edges of the lime rectangle are axis-aligned, so exact compare
/// should succeed with no AA artefacts.
#[test]
fn raster_surface_fill_sub_rect() {
    let (w, h) = (16, 16);
    let buf = with_raster_surface(w, h, |surface| {
        let canvas = surface.canvas();
        canvas.clear(Color::WHITE);

        let mut paint = Paint::new(Color4f::new(0.0, 1.0, 0.0, 1.0), None);
        paint.set_anti_alias(false);
        canvas.draw_rect(Rect::from_xywh(4.0, 4.0, 8.0, 8.0), &paint);

        read_pixels_rgba8(surface)
    });

    // Spot-check: (0,0) white, (4,4) lime, (12,12) lime-edge-inclusive-
    // exclusive depending on rasterizer: just verify centre is lime.
    let px = |x: i32, y: i32| -> &[u8] {
        let i = ((y * w + x) * 4) as usize;
        &buf[i..i + 4]
    };
    assert_eq!(px(0, 0), &[255, 255, 255, 255]);
    assert_eq!(px(8, 8), &[0, 255, 0, 255]);

    assert_matches_golden(
        "smoke_fill_sub_rect",
        w as u32,
        h as u32,
        &buf,
        GoldenCfg::default(),
    );
}

/// Surface created with transparent-black clear must have a fully zero
/// buffer.  Catches any harness-level "cleared to white/black by default"
/// regression.
#[test]
fn raster_surface_default_is_transparent() {
    let buf = with_raster_surface(8, 8, |surface| read_pixels_rgba8(surface));
    assert!(buf.iter().all(|b| *b == 0), "buffer must be all-zero");
}
