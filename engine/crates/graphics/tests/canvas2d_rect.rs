//! Canvas2D rectangle primitives — golden-image tests.
//!
//! Covers:
//!   * `fillRect` solid colour, transparent alpha, globalAlpha modulation
//!   * `strokeRect` line width, cap/join, dash pattern
//!   * `clearRect` (transparent erase regardless of blend mode)
//!
//! All tests use the CPU raster surface from the shared harness so they
//! don't require a GPU context on the CI machine.  Phase 6 will add GPU
//! parallels once the offscreen EGL helper is wired in.

#[path = "common/mod.rs"]
mod common;

use common::golden::{GoldenCfg, assert_matches_golden};
use common::harness::{read_pixels_rgba8, with_raster_surface};

use shared::protocol::color::Color as ProtoColor;
use shared::protocol::render_cmd::Canvas2DCmd::{self, *};

use graphics::backend::gl::canvas::Canvas2DRenderer;
use graphics::backend::gl::paint::NullPatternResolver;

fn apply_all(ctx: &mut Canvas2DRenderer, canvas: &skia_safe::Canvas, cmds: &[Canvas2DCmd]) {
    for c in cmds {
        ctx.apply(canvas, c, &NullPatternResolver);
    }
}

#[test]
fn fill_rect_solid_red_over_white() {
    let (w, h) = (64, 64);
    let buf = with_raster_surface(w, h, |surface| {
        surface.canvas().clear(skia_safe::Color::WHITE);
        let mut ctx = Canvas2DRenderer::new();
        apply_all(
            &mut ctx,
            surface.canvas(),
            &[
                SetFillStyle {
                    color: ProtoColor::rgb(255, 0, 0),
                },
                FillRect {
                    x: 8.0,
                    y: 8.0,
                    w: 48.0,
                    h: 48.0,
                },
            ],
        );
        read_pixels_rgba8(surface)
    });
    assert_matches_golden(
        "canvas2d_fill_rect_solid_red",
        w as u32,
        h as u32,
        &buf,
        GoldenCfg::default(),
    );
}

#[test]
fn fill_rect_alpha_blends_over_background() {
    let (w, h) = (32, 32);
    let buf = with_raster_surface(w, h, |surface| {
        surface.canvas().clear(skia_safe::Color::BLUE);
        let mut ctx = Canvas2DRenderer::new();
        apply_all(
            &mut ctx,
            surface.canvas(),
            &[
                SetFillStyle {
                    color: ProtoColor::rgba(1.0, 0.0, 0.0, 0.5),
                },
                FillRect {
                    x: 0.0,
                    y: 0.0,
                    w: 32.0,
                    h: 32.0,
                },
            ],
        );
        read_pixels_rgba8(surface)
    });
    // Purple-ish due to alpha blend — spot-check centre pixel:
    // src = (255, 0, 0, 128) over dst = (0, 0, 255, 255)
    // result r ≈ 128, g = 0, b ≈ 128, a = 255
    let idx = ((16 * 32 + 16) * 4) as usize;
    assert!((buf[idx] as i32 - 127).abs() <= 2, "r={}", buf[idx]);
    assert_eq!(buf[idx + 1], 0);
    assert!((buf[idx + 2] as i32 - 127).abs() <= 2, "b={}", buf[idx + 2]);
    assert_eq!(buf[idx + 3], 255);
    assert_matches_golden(
        "canvas2d_fill_rect_alpha_blend",
        w as u32,
        h as u32,
        &buf,
        GoldenCfg::default(),
    );
}

#[test]
fn fill_rect_respects_global_alpha() {
    let (w, h) = (16, 16);
    let buf = with_raster_surface(w, h, |surface| {
        surface.canvas().clear(skia_safe::Color::BLACK);
        let mut ctx = Canvas2DRenderer::new();
        apply_all(
            &mut ctx,
            surface.canvas(),
            &[
                SetFillStyle {
                    color: ProtoColor::rgb(255, 255, 255),
                },
                SetGlobalAlpha { alpha: 0.25 },
                FillRect {
                    x: 0.0,
                    y: 0.0,
                    w: 16.0,
                    h: 16.0,
                },
            ],
        );
        read_pixels_rgba8(surface)
    });
    // Expect centre pixel ≈ (64, 64, 64, 255): white × 0.25 over black.
    let idx = ((8 * 16 + 8) * 4) as usize;
    assert!((buf[idx] as i32 - 64).abs() <= 2, "r={}", buf[idx]);
}

#[test]
fn clear_rect_erases_content_to_transparent() {
    let (w, h) = (32, 32);
    let buf = with_raster_surface(w, h, |surface| {
        surface
            .canvas()
            .clear(skia_safe::Color::from_argb(200, 255, 0, 0));
        let mut ctx = Canvas2DRenderer::new();
        apply_all(
            &mut ctx,
            surface.canvas(),
            &[ClearRect {
                x: 8.0,
                y: 8.0,
                w: 16.0,
                h: 16.0,
            }],
        );
        read_pixels_rgba8(surface)
    });
    // Outside the cleared area: original red (premul unmul roundtrip should
    // keep r ~255 and a ~200 in unpremul RGBA).  Inside: fully transparent.
    //
    // Asserted at all four boundaries rather than at one interior pixel. A
    // centre sample says only that *something* was erased somewhere around it:
    // it holds for a rect one pixel to the right, one pixel wider, or one pixel
    // short on any side, and each of those is a visible product defect. The
    // clear paint disables anti-aliasing and these coordinates are integral, so
    // every edge is exact and no tolerance is warranted.
    let alpha = |x: i32, y: i32| buf[((y * w + x) * 4 + 3) as usize];

    assert_eq!(alpha(16, 16), 0, "the middle of the rect is erased");
    for across in 8..24 {
        assert_eq!(alpha(7, across), 200, "the column left of the rect is kept");
        assert_eq!(alpha(8, across), 0, "the rect's first column is erased");
        assert_eq!(alpha(23, across), 0, "the rect's last column is erased");
        assert_eq!(
            alpha(24, across),
            200,
            "the column right of the rect is kept"
        );

        assert_eq!(alpha(across, 7), 200, "the row above the rect is kept");
        assert_eq!(alpha(across, 8), 0, "the rect's first row is erased");
        assert_eq!(alpha(across, 23), 0, "the rect's last row is erased");
        assert_eq!(alpha(across, 24), 200, "the row below the rect is kept");
    }
    assert_eq!(alpha(0, 0), 200, "a corner far from the rect is untouched");
}

#[test]
fn stroke_rect_draws_outline_not_fill() {
    let (w, h) = (32, 32);
    let buf = with_raster_surface(w, h, |surface| {
        surface.canvas().clear(skia_safe::Color::WHITE);
        let mut ctx = Canvas2DRenderer::new();
        apply_all(
            &mut ctx,
            surface.canvas(),
            &[
                SetStrokeStyle {
                    color: ProtoColor::rgb(0, 0, 0),
                },
                SetLineWidth { width: 2.0 },
                StrokeRect {
                    x: 8.0,
                    y: 8.0,
                    w: 16.0,
                    h: 16.0,
                },
            ],
        );
        read_pixels_rgba8(surface)
    });
    // Centre of the rect should still be white (not filled).
    let centre = ((16 * 32 + 16) * 4) as usize;
    assert_eq!(&buf[centre..centre + 4], &[255, 255, 255, 255]);
    assert_matches_golden(
        "canvas2d_stroke_rect_outline",
        w as u32,
        h as u32,
        &buf,
        GoldenCfg::loose_aa(w as u32, h as u32),
    );
}

#[test]
fn composite_source_over_is_the_default() {
    // Validate that drawing a transparent red rect over blue produces the
    // same result whether or not `SetCompositeOperation(0)` is issued.
    let (w, h) = (16, 16);
    let a = with_raster_surface(w, h, |surface| {
        surface.canvas().clear(skia_safe::Color::BLUE);
        let mut ctx = Canvas2DRenderer::new();
        apply_all(
            &mut ctx,
            surface.canvas(),
            &[
                SetFillStyle {
                    color: ProtoColor::rgba(1.0, 0.0, 0.0, 0.5),
                },
                FillRect {
                    x: 0.0,
                    y: 0.0,
                    w: 16.0,
                    h: 16.0,
                },
            ],
        );
        read_pixels_rgba8(surface)
    });
    let b = with_raster_surface(w, h, |surface| {
        surface.canvas().clear(skia_safe::Color::BLUE);
        let mut ctx = Canvas2DRenderer::new();
        apply_all(
            &mut ctx,
            surface.canvas(),
            &[
                SetCompositeOperation { op: 0 },
                SetFillStyle {
                    color: ProtoColor::rgba(1.0, 0.0, 0.0, 0.5),
                },
                FillRect {
                    x: 0.0,
                    y: 0.0,
                    w: 16.0,
                    h: 16.0,
                },
            ],
        );
        read_pixels_rgba8(surface)
    });
    assert_eq!(a, b, "explicit source-over must match default");
}

#[test]
fn save_restore_rewinds_fill_style() {
    let (w, h) = (8, 8);
    let buf = with_raster_surface(w, h, |surface| {
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        let mut ctx = Canvas2DRenderer::new();
        apply_all(
            &mut ctx,
            surface.canvas(),
            &[
                SetFillStyle {
                    color: ProtoColor::rgb(255, 0, 0),
                },
                Save,
                SetFillStyle {
                    color: ProtoColor::rgb(0, 255, 0),
                },
                Restore,
                FillRect {
                    x: 0.0,
                    y: 0.0,
                    w: 8.0,
                    h: 8.0,
                },
            ],
        );
        read_pixels_rgba8(surface)
    });
    let idx = 0;
    // Must be red, not green.
    assert_eq!(&buf[idx..idx + 4], &[255, 0, 0, 255]);
}
