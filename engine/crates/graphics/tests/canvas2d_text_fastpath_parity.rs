//! Pixel parity between the `SkTextBlob` fast path and the SkParagraph path.
//!
//! The fast path exists to skip HarfBuzz shaping and ICU line-break analysis
//! for ordinary UI labels, which are the overwhelming majority of Canvas2D text
//! in small-game code. It is only allowed to do that if it is *indistinguishable*
//! from the complete path -- so this file renders the same call both ways and
//! compares the buffers, rather than checking each way against a golden it
//! could drift from independently.
//!
//! This is the verification the fast path was waiting on. It sat disabled with
//! a comment saying pixel parity across "every font / size / baseline
//! combination our goldens cover" had not been confirmed; the CPU raster
//! harness makes confirming it a host test, so the reason to keep it off has
//! expired rather than been waived.

#[path = "common/mod.rs"]
mod common;

use common::harness::{read_pixels_rgba8, with_raster_surface};

use shared::protocol::color::Color as ProtoColor;
use shared::protocol::render_cmd::Canvas2DCmd::{self, *};
use shared::protocol::render_cmd::{TextAlign, TextBaseline};

use graphics::backend::gl::canvas::{Canvas2DRenderer, DrawEnv};
use graphics::backend::gl::paint::NullPatternResolver;
use graphics::backend::gl::text::TextContext;

const NOTO_SANS: &[u8] = include_bytes!("fixtures/fonts/NotoSans-Regular.ttf");
const W: i32 = 220;
const H: i32 = 96;

/// One text scenario, rendered identically by both paths.
struct Case {
    name: &'static str,
    text: &'static str,
    size: f32,
    align: TextAlign,
    baseline: TextBaseline,
    alpha: f32,
    /// Applied as a canvas transform before the text is drawn.
    scale: Option<(f32, f32)>,
    rotate: Option<f32>,
    /// `maxWidth`, which the paragraph honours as a post-layout scale.
    max_width: f32,
    /// Draw a shadow, which a blob drawn with the same paint would lose.
    shadow: bool,
    /// Use `strokeText` instead of `fillText`.
    stroke: bool,
    /// Whether the blob path is expected to serve this case.
    ///
    /// Both answers are asserted. A case that must be served and is not makes
    /// the pixel comparison vacuous; a case that must not be served and is
    /// means an eligibility rule stopped holding.
    fast: bool,
}

impl Case {
    const fn plain(name: &'static str, text: &'static str, size: f32) -> Self {
        Self {
            name,
            text,
            size,
            align: TextAlign::Start,
            baseline: TextBaseline::Alphabetic,
            alpha: 1.0,
            scale: None,
            rotate: None,
            max_width: f32::INFINITY,
            shadow: false,
            stroke: false,
            fast: true,
        }
    }
}

/// Returns the rendered pixels and how many `fillText` calls the blob path
/// actually served, so a test can tell "identical because equivalent" apart
/// from "identical because the fast path declined everything".
fn render_counted(case: &Case, fast_path: bool) -> (Vec<u8>, u64) {
    with_raster_surface(W, H, |surface| {
        surface.canvas().clear(skia_safe::Color::WHITE);
        let mut text = TextContext::new();
        assert!(text.register_family("test-noto", NOTO_SANS));
        text.set_blob_fast_path(fast_path);

        let resolver = NullPatternResolver;
        let env = DrawEnv {
            canvas: surface.canvas(),
            text: Some(&text),
            resolver: &resolver,
        };
        let mut ctx = Canvas2DRenderer::new();
        ctx.state.text.families =
            std::sync::Arc::new(vec!["test-noto".into(), "sans-serif".into()]);
        ctx.state.text.size = case.size;
        ctx.state.text.align = case.align;
        ctx.state.text.baseline = case.baseline;

        let mut cmds: Vec<Canvas2DCmd> = Vec::new();
        if let Some((sx, sy)) = case.scale {
            cmds.push(Scale { x: sx, y: sy });
        }
        if let Some(radians) = case.rotate {
            cmds.push(Rotate { angle: radians });
        }
        cmds.push(SetGlobalAlpha { alpha: case.alpha });
        cmds.push(SetFillStyle {
            color: ProtoColor::rgb(0, 0, 0),
        });
        if case.shadow {
            cmds.push(SetShadowColor {
                color: ProtoColor::rgb(255, 0, 0),
            });
            cmds.push(SetShadowBlur { blur: 4.0 });
            cmds.push(SetShadowOffsetX { offset: 2.0 });
            cmds.push(SetShadowOffsetY { offset: 2.0 });
        }
        if case.stroke {
            cmds.push(SetStrokeStyle {
                color: ProtoColor::rgb(0, 0, 0),
            });
            cmds.push(SetLineWidth { width: 1.5 });
            cmds.push(StrokeText {
                text: case.text.into(),
                x: 40.0,
                y: 48.0,
                max_width: case.max_width,
            });
        } else {
            cmds.push(FillText {
                text: case.text.into(),
                x: 40.0,
                y: 48.0,
                max_width: case.max_width,
            });
        }

        for c in &cmds {
            ctx.apply_env(&env, c);
        }
        (read_pixels_rgba8(surface), text.fast_path_paint_count())
    })
}

fn render(case: &Case, fast_path: bool) -> Vec<u8> {
    render_counted(case, fast_path).0
}

/// Largest per-channel difference and how many pixels differ at all.
fn compare(a: &[u8], b: &[u8]) -> (u8, usize) {
    assert_eq!(a.len(), b.len());
    let mut worst = 0u8;
    let mut differing = 0usize;
    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let mut pixel_differs = false;
        for (ca, cb) in pa.iter().zip(pb.iter()) {
            let delta = ca.abs_diff(*cb);
            if delta > 0 {
                pixel_differs = true;
                worst = worst.max(delta);
            }
        }
        if pixel_differs {
            differing += 1;
        }
    }
    (worst, differing)
}

fn cases() -> Vec<Case> {
    let mut cases = vec![
        Case::plain("ascii_small", "Score: 1200", 12.0),
        Case::plain("ascii_medium", "Level 3", 20.0),
        Case::plain("ascii_large", "GO!", 40.0),
        Case::plain("digits", "0123456789", 18.0),
        Case::plain("punctuation", "a,b.c;d!e?", 18.0),
        Case::plain("single_char", "X", 24.0),
        Case::plain("spaces", "a b  c", 18.0),
    ];

    for (name, align) in [
        ("align_start", TextAlign::Start),
        ("align_left", TextAlign::Left),
        ("align_center", TextAlign::Center),
        ("align_right", TextAlign::Right),
        ("align_end", TextAlign::End),
    ] {
        cases.push(Case {
            align,
            ..Case::plain(name, "Anchor", 20.0)
        });
    }

    // Only the alphabetic baseline is eligible; the rest must render
    // identically *because they fall back*, which is asserted too.
    for (name, baseline) in [
        ("baseline_top", TextBaseline::Top),
        ("baseline_hanging", TextBaseline::Hanging),
        ("baseline_middle", TextBaseline::Middle),
        ("baseline_alphabetic", TextBaseline::Alphabetic),
        ("baseline_ideographic", TextBaseline::Ideographic),
        ("baseline_bottom", TextBaseline::Bottom),
    ] {
        cases.push(Case {
            baseline,
            fast: baseline == TextBaseline::Alphabetic,
            ..Case::plain(name, "Baseline", 20.0)
        });
    }

    cases.push(Case {
        alpha: 0.35,
        ..Case::plain("alpha", "Faded", 22.0)
    });
    cases.push(Case {
        scale: Some((1.75, 1.75)),
        ..Case::plain("scaled", "Big", 14.0)
    });
    cases.push(Case {
        rotate: Some(0.2),
        fast: false,
        ..Case::plain("rotated", "Tilt", 20.0)
    });
    cases
}

#[test]
fn the_fast_path_is_pixel_identical_to_the_paragraph_path() {
    let mut failures = Vec::new();
    for case in cases() {
        let paragraph = render(&case, false);
        let (blob, served) = render_counted(&case, true);

        let expected_paints = u64::from(case.fast);
        assert_eq!(
            served, expected_paints,
            "case {:?}: expected the blob path to serve {expected_paints} paint(s), \
             it served {served}",
            case.name
        );

        // A case that renders nothing proves nothing: if both paths drew an
        // empty surface the comparison would pass without exercising anything.
        let ink = paragraph
            .chunks_exact(4)
            .filter(|p| p[0] < 250 || p[1] < 250 || p[2] < 250)
            .count();
        assert!(
            ink > 10,
            "case {:?} drew almost nothing ({ink} non-white pixels); the parity \
             comparison would be vacuous",
            case.name
        );

        let (worst, differing) = compare(&paragraph, &blob);
        if worst != 0 {
            failures.push(format!(
                "{}: {differing} pixel(s) differ, worst channel delta {worst}",
                case.name
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "the SkTextBlob path must be indistinguishable from SkParagraph:\n  {}",
        failures.join("\n  ")
    );
}

#[test]
fn ineligible_text_still_goes_through_the_paragraph_path() {
    // Each of these is a case the blob cannot reproduce. Rendering with the
    // fast path enabled must produce exactly what the paragraph path produces,
    // because the fast path must have declined it.
    let ineligible = [
        Case::plain("non_ascii", "héllo wörld", 20.0),
        Case::plain("empty", "", 20.0),
        Case {
            shadow: true,
            ..Case::plain("shadow", "Shadowed", 20.0)
        },
        Case {
            stroke: true,
            ..Case::plain("stroke", "Outlined", 20.0)
        },
        Case {
            max_width: 40.0,
            ..Case::plain("max_width_scales", "Squeezed", 20.0)
        },
    ];
    for case in ineligible {
        let paragraph = render(&case, false);
        let (blob, served) = render_counted(&case, true);
        assert_eq!(
            served, 0,
            "{}: the fast path served this despite being ineligible",
            case.name
        );
        if !case.text.is_empty() {
            let ink = paragraph
                .chunks_exact(4)
                .filter(|p| p[0] < 250 || p[1] < 250 || p[2] < 250)
                .count();
            assert!(
                ink > 10,
                "case {:?} drew almost nothing; \"the fast path declined it\" \
                 would be true of a blank surface too",
                case.name
            );
        }
        let (worst, differing) = compare(&paragraph, &blob);
        assert_eq!(
            worst, 0,
            "{}: fast path changed {differing} pixel(s) on text it must decline",
            case.name
        );
    }
}
