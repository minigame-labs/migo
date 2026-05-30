//! Canvas2D text-layout attributes ↔ Skia mapping.
//!
//! Maps the HTML `CanvasTextAlign` / `CanvasTextBaseline` enumerations to the
//! corresponding Skia primitives.  Intentionally kept in a separate, GL-free
//! module so the state machine can be unit-tested without a GPU context.
//!
//! Direction handling:
//!   Canvas spec's `"start"` and `"end"` values depend on the canvas element
//!   writing-mode (LTR vs RTL).  We resolve them to concrete `Left`/`Right`
//!   values here with the caller supplying the current direction — the
//!   renderer does not track writing-mode state, so this is always `LTR`
//!   for now.  (Future i18n enhancement: propagate direction through the
//!   canvas element attribute.)

use shared::protocol::render_cmd::{TextAlign, TextBaseline};
use skia_safe::textlayout::{TextAlign as SkTextAlign, TextDirection};

/// A concrete horizontal alignment with `start`/`end` already resolved
/// against a writing direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedTextAlign {
    Left,
    Right,
    Center,
}

impl ResolvedTextAlign {
    /// Resolve a Canvas2D `TextAlign` under the given writing direction.
    ///
    /// Per WHATWG Canvas 2D spec, `start` means the "beginning of the line"
    /// and `end` means the "end of the line" in the paragraph's base
    /// direction.  `left`/`right`/`center` are absolute and ignore direction.
    pub fn resolve(align: TextAlign, direction: TextDirection) -> Self {
        match align {
            TextAlign::Left => Self::Left,
            TextAlign::Right => Self::Right,
            TextAlign::Center => Self::Center,
            TextAlign::Start => match direction {
                TextDirection::LTR => Self::Left,
                TextDirection::RTL => Self::Right,
            },
            TextAlign::End => match direction {
                TextDirection::LTR => Self::Right,
                TextDirection::RTL => Self::Left,
            },
        }
    }

    /// Convert to Skia's paragraph-level `TextAlign` enum.
    pub fn to_sk(self) -> SkTextAlign {
        match self {
            Self::Left => SkTextAlign::Left,
            Self::Right => SkTextAlign::Right,
            Self::Center => SkTextAlign::Center,
        }
    }

    /// Horizontal shift (in pixels) to apply to a run of width `w` so that
    /// the draw origin `(x, y)` matches Canvas2D semantics.
    ///
    /// Canvas2D draws text with `(x, y)` as the anchor — for `left` the
    /// anchor is the left edge, for `center` the midpoint, for `right` the
    /// right edge.  Skia's `Paragraph::paint` always paints with `(x, y)`
    /// as the *top-left* of the measured run, so we subtract this offset
    /// from `x` before calling `paint`.
    #[inline]
    pub fn x_anchor_offset(self, measured_width: f32) -> f32 {
        match self {
            Self::Left => 0.0,
            Self::Right => measured_width,
            Self::Center => measured_width * 0.5,
        }
    }
}

/// Vertical baseline offset in pixels, given a font's ascent/descent.
///
/// Canvas2D spec normatively defines six baselines.  We reduce to a single
/// numeric offset that is added to the draw `y` — the offset transforms the
/// caller's anchor Y into the top-of-paragraph Y that Skia expects.
///
/// `ascent` is positive-above-baseline (as Skia reports it),
/// `descent` is positive-below-baseline.
#[inline]
pub fn y_baseline_offset(baseline: TextBaseline, ascent: f32, descent: f32) -> f32 {
    // Returns the offset such that: paragraph_top_y = caller_y - offset
    match baseline {
        TextBaseline::Top => 0.0,
        TextBaseline::Hanging => ascent * 0.8, // HTML: hanging ≈ 0.8 of ascent
        TextBaseline::Middle => ascent + (descent - ascent) * 0.5,
        TextBaseline::Alphabetic => ascent,
        TextBaseline::Ideographic => ascent + descent * 0.5,
        TextBaseline::Bottom => ascent + descent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_ltr_is_left() {
        assert_eq!(
            ResolvedTextAlign::resolve(TextAlign::Start, TextDirection::LTR),
            ResolvedTextAlign::Left
        );
    }

    #[test]
    fn end_ltr_is_right() {
        assert_eq!(
            ResolvedTextAlign::resolve(TextAlign::End, TextDirection::LTR),
            ResolvedTextAlign::Right
        );
    }

    #[test]
    fn start_rtl_is_right() {
        assert_eq!(
            ResolvedTextAlign::resolve(TextAlign::Start, TextDirection::RTL),
            ResolvedTextAlign::Right
        );
    }

    #[test]
    fn end_rtl_is_left() {
        assert_eq!(
            ResolvedTextAlign::resolve(TextAlign::End, TextDirection::RTL),
            ResolvedTextAlign::Left
        );
    }

    #[test]
    fn absolute_aligns_ignore_direction() {
        for dir in [TextDirection::LTR, TextDirection::RTL] {
            assert_eq!(
                ResolvedTextAlign::resolve(TextAlign::Left, dir),
                ResolvedTextAlign::Left
            );
            assert_eq!(
                ResolvedTextAlign::resolve(TextAlign::Right, dir),
                ResolvedTextAlign::Right
            );
            assert_eq!(
                ResolvedTextAlign::resolve(TextAlign::Center, dir),
                ResolvedTextAlign::Center
            );
        }
    }

    #[test]
    fn x_anchor_offset_matches_spec() {
        let w = 100.0_f32;
        assert_eq!(ResolvedTextAlign::Left.x_anchor_offset(w), 0.0);
        assert_eq!(ResolvedTextAlign::Center.x_anchor_offset(w), 50.0);
        assert_eq!(ResolvedTextAlign::Right.x_anchor_offset(w), 100.0);
    }

    #[test]
    fn x_anchor_offset_handles_zero_width() {
        let w = 0.0_f32;
        for a in [
            ResolvedTextAlign::Left,
            ResolvedTextAlign::Center,
            ResolvedTextAlign::Right,
        ] {
            assert_eq!(a.x_anchor_offset(w), 0.0);
        }
    }

    // Baseline tests use: ascent=12, descent=4 → total line height 16
    // We're verifying the anchor offset math, not perfect CSS parity.
    #[test]
    fn baseline_top_is_zero_offset() {
        assert_eq!(y_baseline_offset(TextBaseline::Top, 12.0, 4.0), 0.0);
    }

    #[test]
    fn baseline_alphabetic_is_full_ascent() {
        assert_eq!(y_baseline_offset(TextBaseline::Alphabetic, 12.0, 4.0), 12.0);
    }

    #[test]
    fn baseline_bottom_is_ascent_plus_descent() {
        assert_eq!(y_baseline_offset(TextBaseline::Bottom, 12.0, 4.0), 16.0);
    }

    #[test]
    fn baseline_middle_is_between_ascent_and_descent() {
        // With ascent=12 descent=4: middle = 12 + (4-12)*0.5 = 12 - 4 = 8
        assert_eq!(y_baseline_offset(TextBaseline::Middle, 12.0, 4.0), 8.0);
    }

    #[test]
    fn baseline_ideographic_is_below_alphabetic() {
        let alpha = y_baseline_offset(TextBaseline::Alphabetic, 12.0, 4.0);
        let ideo = y_baseline_offset(TextBaseline::Ideographic, 12.0, 4.0);
        assert!(
            ideo > alpha,
            "ideographic {ideo} must sit below alphabetic {alpha}"
        );
    }

    #[test]
    fn baseline_hanging_is_above_alphabetic() {
        let alpha = y_baseline_offset(TextBaseline::Alphabetic, 12.0, 4.0);
        let hang = y_baseline_offset(TextBaseline::Hanging, 12.0, 4.0);
        assert!(
            hang < alpha,
            "hanging {hang} must sit above alphabetic {alpha}"
        );
    }

    #[test]
    fn resolved_to_sk_maps_directly() {
        assert_eq!(ResolvedTextAlign::Left.to_sk(), SkTextAlign::Left);
        assert_eq!(ResolvedTextAlign::Right.to_sk(), SkTextAlign::Right);
        assert_eq!(ResolvedTextAlign::Center.to_sk(), SkTextAlign::Center);
    }
}
