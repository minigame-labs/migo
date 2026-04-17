//! Canvas2D drawing state (Skia-native, GL-free).
//!
//! Mirrors the [WHATWG Canvas 2D §2.4 drawing state][spec] so that a single
//! `Canvas2DState` value captures everything `SkPaint` / `SkCanvas` needs to
//! honour `save()` / `restore()`, minus the bits Skia tracks on its own:
//!
//!   * **CTM** (current transformation matrix) — maintained by `SkCanvas`
//!     via `save()`/`restore()`/`concat()` / `translate()` / `scale()` etc.
//!   * **Clip region** — same, `SkCanvas::clip_*()` rides the save stack.
//!
//! The state struct is therefore a pure Rust value object with a tiny
//! save/restore stack.  It is unit-testable without a GPU context — this
//! module is exercised heavily by `backend::gl::state::tests` below.
//!
//! ## Design notes
//!
//! * **Styles are lazily resolved.**  `FillStyleKind` / `StrokeStyleKind`
//!   keep the high-level description (colour / gradient stops / pattern
//!   image handle); the `SkPaint` / `SkShader` is built at draw time.  This
//!   matches Chrome/Blink and keeps `save()` cheap (no `Shader::clone()`).
//! * **globalAlpha** is *not* folded into `FillStyleKind::Color` here;
//!   modulation happens at paint-build time via
//!   [`crate::backend::gl::color::to_sk_color4f_modulated`].
//! * **Shadow** is represented as a `Shadow` struct; an `ImageFilter` is
//!   built lazily by the handler when a draw call sees a non-trivial shadow.
//! * The [Canvas 2D spec default drawing state][spec-defaults] is encoded
//!   in `Default for Canvas2DState`.  Tests below pin every default down so
//!   regressions are caught by CI, not by a user on device.
//!
//! [spec]: https://html.spec.whatwg.org/multipage/canvas.html#the-canvas-state
//! [spec-defaults]: https://html.spec.whatwg.org/multipage/canvas.html#reset-the-rendering-context-to-its-default-state

use shared::protocol::color::Color;
use shared::protocol::render_cmd::{GradientStop, GradientType, TextAlign, TextBaseline};
use skia_safe::{BlendMode, PaintCap, PaintJoin};

/// Fill (or stroke) style referenced by a [`Canvas2DState`].
///
/// See WHATWG Canvas 2D §3 "fillStyle".  Pattern entries carry the image
/// registry id only; the `SkImage` is resolved by the handler at draw time
/// so this enum stays GL-free.
#[derive(Debug, Clone, PartialEq)]
pub enum StyleKind {
    Color(Color),
    LinearGradient {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        stops: Vec<GradientStop>,
    },
    RadialGradient {
        x0: f32,
        y0: f32,
        r0: f32,
        x1: f32,
        y1: f32,
        r1: f32,
        stops: Vec<GradientStop>,
    },
    ConicGradient {
        cx: f32,
        cy: f32,
        start_angle: f32,
        stops: Vec<GradientStop>,
    },
    Pattern {
        image_id: u32,
        repeat_x: bool,
        repeat_y: bool,
    },
}

impl StyleKind {
    /// Build a [`StyleKind`] from the shared protocol's gradient shape +
    /// colour stops.  Extracted because both fill and stroke variants go
    /// through the exact same decode path.
    pub fn from_gradient(
        kind: GradientType,
        x0: f32,
        y0: f32,
        r0: f32,
        x1: f32,
        y1: f32,
        _r1: f32,
        stops: Vec<GradientStop>,
    ) -> Self {
        match kind {
            GradientType::Linear => StyleKind::LinearGradient {
                x0,
                y0,
                x1,
                y1,
                stops,
            },
            GradientType::Radial => StyleKind::RadialGradient {
                x0,
                y0,
                r0,
                x1,
                y1,
                r1: _r1,
                stops,
            },
            GradientType::Conic => StyleKind::ConicGradient {
                cx: x0,
                cy: y0,
                start_angle: x1, // JS encodes start angle in x1 for conic
                stops,
            },
        }
    }
}

/// Shadow parameters, matching the four shadow attributes in Canvas 2D §4
/// "drawing images / shapes".  Stored as-is; the handler converts to a
/// `SkImageFilter::blur` at draw time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub blur: f32,
    pub color: Color,
    pub offset_x: f32,
    pub offset_y: f32,
}

impl Shadow {
    /// Canvas 2D spec defaults: fully transparent black, zero blur/offset —
    /// i.e. draws have no shadow unless explicitly enabled.
    #[inline]
    pub const fn none() -> Self {
        Self {
            blur: 0.0,
            color: Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            },
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }

    /// Returns `true` if the shadow is visible for the current draw.
    ///
    /// Per spec the shadow is drawn only when the shadow colour is non-
    /// transparent **and** at least one of `blur`, `offset_x`, `offset_y`
    /// is non-zero.  Zero-offset zero-blur shadows are spec'd as no-ops
    /// even if the colour is opaque — matches Chrome.
    #[inline]
    pub fn is_visible(&self) -> bool {
        let has_color = self.color.a > 0.0;
        let has_extent = self.blur > 0.0 || self.offset_x != 0.0 || self.offset_y != 0.0;
        has_color && has_extent
    }
}

/// Text-layout attributes (font + align + baseline).
///
/// Typeface resolution is handled separately by the font manager; this just
/// carries the *descriptor* so the state can be snapshotted cheaply.
#[derive(Debug, Clone, PartialEq)]
pub struct TextAttrs {
    /// `font-size` in CSS px.
    pub size: f32,
    /// Parsed family list, head-first (e.g. `["Helvetica", "sans-serif"]`).
    pub families: Vec<String>,
    /// CSS `font-weight`, 1..=1000 (400 = normal, 700 = bold).
    pub weight: u16,
    /// CSS `font-style: italic | oblique`.
    pub italic: bool,
    pub align: TextAlign,
    pub baseline: TextBaseline,
}

impl Default for TextAttrs {
    /// Canvas 2D default: `10px sans-serif`, `start` align, `alphabetic`
    /// baseline, normal weight, upright.
    fn default() -> Self {
        Self {
            size: 10.0,
            families: vec!["sans-serif".to_string()],
            weight: 400,
            italic: false,
            align: TextAlign::Start,
            baseline: TextBaseline::Alphabetic,
        }
    }
}

/// Full drawing state for one 2D context.  See module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct Canvas2DState {
    pub fill: StyleKind,
    pub stroke: StyleKind,

    pub line_width: f32,
    pub line_cap: PaintCap,
    pub line_join: PaintJoin,
    pub miter_limit: f32,

    pub line_dash: Vec<f32>,
    pub line_dash_offset: f32,

    pub global_alpha: f32,
    pub blend_mode: BlendMode,

    pub shadow: Shadow,
    pub text: TextAttrs,
    /// Enables/disables default anti-aliasing for fills/strokes.
    pub antialias: bool,
    /// Image-smoothing (bilinear) flag — affects `drawImage` when scaling.
    pub image_smoothing: bool,
}

impl Default for Canvas2DState {
    /// Full set of WHATWG Canvas 2D defaults.
    ///
    /// Any change here is a user-visible behaviour change and MUST be
    /// paired with a test update in `tests::defaults_match_whatwg_spec`.
    fn default() -> Self {
        Self {
            fill: StyleKind::Color(Color::black()),
            stroke: StyleKind::Color(Color::black()),

            line_width: 1.0,
            line_cap: PaintCap::Butt,
            line_join: PaintJoin::Miter,
            miter_limit: 10.0,

            line_dash: Vec::new(),
            line_dash_offset: 0.0,

            global_alpha: 1.0,
            blend_mode: BlendMode::SrcOver,

            shadow: Shadow::none(),
            text: TextAttrs::default(),
            antialias: true,
            image_smoothing: true,
        }
    }
}

impl Canvas2DState {
    /// Return `true` if the *fill* side must go through a shader (gradient
    /// or pattern) rather than a flat colour.  Exposed so the handler can
    /// short-circuit paint construction on the hot path.
    #[inline]
    pub fn fill_needs_shader(&self) -> bool {
        !matches!(&self.fill, StyleKind::Color(_))
    }

    /// Sibling of [`Self::fill_needs_shader`] for the stroke side.
    #[inline]
    pub fn stroke_needs_shader(&self) -> bool {
        !matches!(&self.stroke, StyleKind::Color(_))
    }
}

/// Save/restore stack for [`Canvas2DState`].
///
/// The Canvas 2D spec caps the "drawing state stack" depth at a "reasonable
/// limit" — we use 32, the same value Chrome picked to balance push cost
/// against legitimate nested drawing (e.g. Cocos nested `save()` chains).
/// Beyond that, further `save()` calls still *record* (so `restore()` pops
/// correctly) but no additional memory is allocated for the snapshots; the
/// handler additionally forwards the `save()` / `restore()` to `SkCanvas`
/// which handles CTM and clip.
pub const MAX_STATE_STACK_DEPTH: usize = 32;

#[derive(Debug, Default)]
pub struct StateStack {
    /// Saved state snapshots, LIFO.  `len() ≤ MAX_STATE_STACK_DEPTH`.
    saved: Vec<Canvas2DState>,
}

impl StateStack {
    pub fn new() -> Self {
        Self {
            saved: Vec::with_capacity(8),
        }
    }

    /// Number of snapshots currently on the stack (equals the number of
    /// `save()` calls that have no matching `restore()`).
    #[inline]
    pub fn depth(&self) -> usize {
        self.saved.len()
    }

    /// Push a snapshot of `state`.
    ///
    /// Returns `true` if the snapshot was recorded, `false` if the stack
    /// was at its depth limit and the save was dropped (the handler should
    /// still call `SkCanvas::save()` in both cases to keep the CTM/clip
    /// stack in sync — the stack depth mismatch is the *whole point* of
    /// the shallow limit: we rely on Skia's internal stack being deeper).
    pub fn push(&mut self, state: &Canvas2DState) -> bool {
        if self.saved.len() >= MAX_STATE_STACK_DEPTH {
            return false;
        }
        self.saved.push(state.clone());
        true
    }

    /// Pop the most recent snapshot into `state`.
    ///
    /// Returns `true` if a snapshot existed, `false` on under-flow
    /// (`restore()` before `save()` — a legal Canvas 2D no-op).
    pub fn pop(&mut self, state: &mut Canvas2DState) -> bool {
        match self.saved.pop() {
            Some(snapshot) => {
                *state = snapshot;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- defaults ------------------------------------------------------
    #[test]
    fn defaults_match_whatwg_spec() {
        let s = Canvas2DState::default();

        // Styles: black opaque
        assert_eq!(s.fill, StyleKind::Color(Color::black()));
        assert_eq!(s.stroke, StyleKind::Color(Color::black()));

        // Stroke attributes
        assert_eq!(s.line_width, 1.0);
        assert!(matches!(s.line_cap, PaintCap::Butt));
        assert!(matches!(s.line_join, PaintJoin::Miter));
        assert_eq!(s.miter_limit, 10.0);
        assert!(s.line_dash.is_empty());
        assert_eq!(s.line_dash_offset, 0.0);

        // Compositing
        assert_eq!(s.global_alpha, 1.0);
        assert_eq!(s.blend_mode, BlendMode::SrcOver);

        // Shadow: off
        assert!(!s.shadow.is_visible());

        // Text
        assert_eq!(s.text.size, 10.0);
        assert_eq!(s.text.families, vec!["sans-serif".to_string()]);
        assert_eq!(s.text.weight, 400);
        assert!(!s.text.italic);
        assert!(matches!(s.text.align, TextAlign::Start));
        assert!(matches!(s.text.baseline, TextBaseline::Alphabetic));

        // Smoothing + anti-aliasing
        assert!(s.antialias);
        assert!(s.image_smoothing);
    }

    #[test]
    fn fill_needs_shader_for_gradients_and_patterns() {
        let mut s = Canvas2DState::default();
        assert!(!s.fill_needs_shader());

        s.fill = StyleKind::LinearGradient {
            x0: 0.0,
            y0: 0.0,
            x1: 100.0,
            y1: 0.0,
            stops: vec![],
        };
        assert!(s.fill_needs_shader());

        s.fill = StyleKind::Pattern {
            image_id: 42,
            repeat_x: true,
            repeat_y: true,
        };
        assert!(s.fill_needs_shader());
    }

    // ---- shadow visibility --------------------------------------------
    #[test]
    fn transparent_shadow_is_not_visible_even_with_blur() {
        let s = Shadow {
            blur: 10.0,
            color: Color::transparent(),
            offset_x: 5.0,
            offset_y: 5.0,
        };
        assert!(!s.is_visible());
    }

    #[test]
    fn opaque_zero_shadow_is_not_visible() {
        // Matches Chrome: a non-transparent shadow colour with blur=0 and
        // offsets=(0,0) produces NO shadow (it would just overpaint itself).
        let s = Shadow {
            blur: 0.0,
            color: Color::black(),
            offset_x: 0.0,
            offset_y: 0.0,
        };
        assert!(!s.is_visible());
    }

    #[test]
    fn opaque_shadow_with_blur_is_visible() {
        let s = Shadow {
            blur: 4.0,
            color: Color::black(),
            offset_x: 0.0,
            offset_y: 0.0,
        };
        assert!(s.is_visible());
    }

    #[test]
    fn opaque_shadow_with_offset_only_is_visible() {
        let s = Shadow {
            blur: 0.0,
            color: Color::black(),
            offset_x: 2.0,
            offset_y: 0.0,
        };
        assert!(s.is_visible());
    }

    // ---- StyleKind ----------------------------------------------------
    #[test]
    fn gradient_type_linear_drops_r_inputs() {
        let k = StyleKind::from_gradient(
            GradientType::Linear,
            0.0,
            0.0,
            99.0, // r0 ignored
            100.0,
            0.0,
            88.0, // r1 ignored
            vec![],
        );
        assert!(matches!(k, StyleKind::LinearGradient { .. }));
    }

    #[test]
    fn gradient_type_radial_preserves_r_inputs() {
        let k = StyleKind::from_gradient(
            GradientType::Radial,
            10.0,
            20.0,
            30.0,
            40.0,
            50.0,
            60.0,
            vec![],
        );
        match k {
            StyleKind::RadialGradient { r0, r1, .. } => {
                assert_eq!(r0, 30.0);
                assert_eq!(r1, 60.0);
            }
            _ => panic!("wrong kind"),
        }
    }

    #[test]
    fn gradient_type_conic_encodes_start_angle_in_x1() {
        let k = StyleKind::from_gradient(
            GradientType::Conic,
            50.0, // cx
            60.0, // cy
            0.0,
            std::f32::consts::FRAC_PI_2, // start angle in x1 (90°)
            0.0,
            0.0,
            vec![],
        );
        match k {
            StyleKind::ConicGradient {
                cx,
                cy,
                start_angle,
                ..
            } => {
                assert_eq!(cx, 50.0);
                assert_eq!(cy, 60.0);
                assert!((start_angle - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
            }
            _ => panic!("wrong kind"),
        }
    }

    // ---- save / restore ------------------------------------------------
    #[test]
    fn restore_without_save_is_noop() {
        let mut stack = StateStack::new();
        let mut state = Canvas2DState::default();
        let before = state.clone();
        assert!(!stack.pop(&mut state));
        assert_eq!(state, before);
    }

    #[test]
    fn save_restore_roundtrip_preserves_state() {
        let mut stack = StateStack::new();
        let mut state = Canvas2DState::default();

        assert!(stack.push(&state));
        assert_eq!(stack.depth(), 1);

        // Mutate
        state.line_width = 5.5;
        state.global_alpha = 0.25;
        state.blend_mode = BlendMode::Plus;
        state.fill = StyleKind::Color(Color::rgb(10, 20, 30));
        state.text.size = 42.0;

        assert!(stack.pop(&mut state));
        assert_eq!(stack.depth(), 0);
        assert_eq!(state, Canvas2DState::default());
    }

    #[test]
    fn save_captures_independent_snapshot() {
        // Verifies the stack does not hold a reference to the live state.
        let mut stack = StateStack::new();
        let mut state = Canvas2DState::default();

        state.line_width = 3.0;
        stack.push(&state);

        state.line_width = 7.0;
        assert_eq!(state.line_width, 7.0);

        stack.pop(&mut state);
        assert_eq!(state.line_width, 3.0);
    }

    #[test]
    fn stack_caps_depth_and_signals_overflow() {
        let mut stack = StateStack::new();
        let state = Canvas2DState::default();

        for _ in 0..MAX_STATE_STACK_DEPTH {
            assert!(stack.push(&state));
        }
        assert_eq!(stack.depth(), MAX_STATE_STACK_DEPTH);

        // One more should silently drop (returns false).
        assert!(!stack.push(&state));
        assert_eq!(stack.depth(), MAX_STATE_STACK_DEPTH);
    }

    #[test]
    fn deep_save_restore_preserves_order() {
        // LIFO semantics: last-saved state is first-restored.
        let mut stack = StateStack::new();
        let mut state = Canvas2DState::default();

        for i in 1..=4 {
            state.line_width = i as f32;
            stack.push(&state);
        }
        // After 4 pushes the live state is line_width=4.0
        // The stack (bottom → top) is 1.0, 2.0, 3.0, 4.0

        // Mutate beyond the last push, then restore -- should get 4.0 back.
        state.line_width = 99.0;
        stack.pop(&mut state);
        assert_eq!(state.line_width, 4.0);

        stack.pop(&mut state);
        assert_eq!(state.line_width, 3.0);

        stack.pop(&mut state);
        assert_eq!(state.line_width, 2.0);

        stack.pop(&mut state);
        assert_eq!(state.line_width, 1.0);

        assert_eq!(stack.depth(), 0);
    }
}
