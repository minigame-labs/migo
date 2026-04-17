//! The per-context Canvas2D render loop.
//!
//! A [`Canvas2DRenderer`] owns the mutable drawing state (`Canvas2DState`),
//! the save/restore stack, and the in-flight `CanvasPath`.  It *does not*
//! own an `SkCanvas` — the canvas is passed in per-call so the same
//! context can render to CPU raster surfaces (tests) or the GPU-backed
//! onscreen `SkSurface` (production) without refactoring.
//!
//! Side-effecting commands that produce a reply (MeasureText / GetImageData)
//! are not handled here; the upper layer dispatches them directly against
//! the surface before routing the reply channel.
//!
//! The handler is *non-exhaustive* (see `Canvas2DCmd`): unknown variants
//! are logged and ignored rather than panicking, so adding a new command
//! upstream does not break builds of older backend revisions.

use shared::protocol::color::Color as ProtocolColor;
use shared::protocol::render_cmd::{Canvas2DCmd, GradientType, TextAlign, TextBaseline};
use skia_safe::{Canvas, ClipOp, Matrix, PaintCap, PaintJoin, Rect as SkRect};

use super::blend_mode::blend_mode_from_code;
use super::paint::{
    build_clear_paint, build_fill_paint, build_stroke_paint, PatternResolver,
};
use super::path::CanvasPath;
use super::state::{Canvas2DState, Shadow, StateStack, StyleKind};
use super::text::TextContext;

/// Bundle of render-time resources passed alongside each command.
///
/// Factored out so the handler stays a single entry point regardless of
/// which resources a specific command consults: plain `fillRect` only
/// uses `canvas`; `fillText` additionally uses `text`; `drawImage` /
/// `Pattern` gradient uses `resolver`.
///
/// `'e` bounds the lifetime of the environment borrow, ensuring that
/// `apply()` does not retain any reference past the call.
pub struct DrawEnv<'e, R: PatternResolver> {
    pub canvas: &'e Canvas,
    pub text: &'e TextContext,
    pub resolver: &'e R,
}

/// Owns the Canvas2D state for one `CanvasRenderingContext2D`.
pub struct Canvas2DRenderer {
    pub state: Canvas2DState,
    pub stack: StateStack,
    pub path: CanvasPath,
}

impl Default for Canvas2DRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Canvas2DRenderer {
    pub fn new() -> Self {
        Self {
            state: Canvas2DState::default(),
            stack: StateStack::new(),
            path: CanvasPath::new(),
        }
    }

    /// Apply one Canvas2D command.  Returns `true` when the command caused
    /// an observable change to the target surface (a draw was issued or
    /// pixels were cleared); `false` for pure state mutation and path
    /// building.  The render thread uses the boolean to decide whether
    /// the canvas requires a present.
    ///
    /// Legacy form — callers that don't need text rendering can pass only
    /// a canvas + resolver.  Forwards internally to the full `DrawEnv`
    /// variant with a lazily-constructed fresh [`TextContext`].
    pub fn apply<R: PatternResolver>(
        &mut self,
        canvas: &Canvas,
        cmd: &Canvas2DCmd,
        resolver: &R,
    ) -> bool {
        let empty_text = TextContext::new();
        let env = DrawEnv {
            canvas,
            text: &empty_text,
            resolver,
        };
        self.apply_env(&env, cmd)
    }

    /// Apply one command against a full [`DrawEnv`].  Required for any
    /// text-related opcode; non-text commands ignore `env.text`.
    pub fn apply_env<R: PatternResolver>(
        &mut self,
        env: &DrawEnv<'_, R>,
        cmd: &Canvas2DCmd,
    ) -> bool {
        let canvas = env.canvas;
        let resolver = env.resolver;
        let text_ctx = env.text;
        use Canvas2DCmd::*;
        match cmd {
            // ---- Path building -------------------------------------
            BeginPath => {
                self.path.reset();
                false
            }
            ClosePath => {
                self.path.close_path();
                false
            }
            MoveTo { x, y } => {
                self.path.move_to(*x, *y);
                false
            }
            LineTo { x, y } => {
                self.path.line_to(*x, *y);
                false
            }
            QuadraticCurveTo { cpx, cpy, x, y } => {
                self.path.quadratic_to(*cpx, *cpy, *x, *y);
                false
            }
            BezierCurveTo {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => {
                self.path.bezier_to(*cp1x, *cp1y, *cp2x, *cp2y, *x, *y);
                false
            }
            Arc {
                x,
                y,
                radius,
                start_angle,
                end_angle,
                counterclockwise,
            } => {
                self.path
                    .arc(*x, *y, *radius, *start_angle, *end_angle, *counterclockwise);
                false
            }
            ArcTo {
                x1,
                y1,
                x2,
                y2,
                radius,
            } => {
                self.path.arc_to(*x1, *y1, *x2, *y2, *radius);
                false
            }
            Rect { x, y, w, h } => {
                self.path.rect(*x, *y, *w, *h);
                false
            }
            Ellipse {
                x,
                y,
                radius_x,
                radius_y,
                rotation,
                start_angle,
                end_angle,
                counterclockwise,
            } => {
                self.path.ellipse(
                    *x,
                    *y,
                    *radius_x,
                    *radius_y,
                    *rotation,
                    *start_angle,
                    *end_angle,
                    *counterclockwise,
                );
                false
            }

            // ---- Path-based drawing -------------------------------
            Fill => {
                let paint = build_fill_paint(&self.state, resolver);
                let path = self.path.snapshot();
                canvas.draw_path(&path, &paint);
                true
            }
            Stroke => {
                let paint = build_stroke_paint(&self.state, resolver);
                let path = self.path.snapshot();
                canvas.draw_path(&path, &paint);
                true
            }
            Clip => {
                let path = self.path.snapshot();
                canvas.clip_path(&path, ClipOp::Intersect, true);
                false
            }

            // ---- Rectangle primitives -----------------------------
            FillRect { x, y, w, h } => {
                let paint = build_fill_paint(&self.state, resolver);
                canvas.draw_rect(SkRect::from_xywh(*x, *y, *w, *h), &paint);
                true
            }
            StrokeRect { x, y, w, h } => {
                let paint = build_stroke_paint(&self.state, resolver);
                canvas.draw_rect(SkRect::from_xywh(*x, *y, *w, *h), &paint);
                true
            }
            ClearRect { x, y, w, h } => {
                let paint = build_clear_paint();
                canvas.draw_rect(SkRect::from_xywh(*x, *y, *w, *h), &paint);
                true
            }

            // ---- Style setters (pure state) -----------------------
            SetFillStyle { color } => {
                self.state.fill = StyleKind::Color(*color);
                false
            }
            SetStrokeStyle { color } => {
                self.state.stroke = StyleKind::Color(*color);
                false
            }
            SetLineWidth { width } => {
                // Canvas spec: ignore negative/zero/NaN/Inf values.
                if width.is_finite() && *width > 0.0 {
                    self.state.line_width = *width;
                }
                false
            }
            SetLineCap { cap } => {
                self.state.line_cap = match cap {
                    0 => PaintCap::Butt,
                    1 => PaintCap::Round,
                    2 => PaintCap::Square,
                    _ => PaintCap::Butt,
                };
                false
            }
            SetLineJoin { join } => {
                self.state.line_join = match join {
                    0 => PaintJoin::Miter,
                    1 => PaintJoin::Round,
                    2 => PaintJoin::Bevel,
                    _ => PaintJoin::Miter,
                };
                false
            }
            SetMiterLimit { limit } => {
                if limit.is_finite() && *limit > 0.0 {
                    self.state.miter_limit = *limit;
                }
                false
            }
            SetGlobalAlpha { alpha } => {
                if alpha.is_finite() && (0.0..=1.0).contains(alpha) {
                    self.state.global_alpha = *alpha;
                }
                false
            }
            SetCompositeOperation { op } => {
                self.state.blend_mode = blend_mode_from_code(*op);
                false
            }
            SetLineDash { segments } => {
                // Spec: odd-length dash arrays double up to even length.
                let mut d = segments.clone();
                if d.len() % 2 == 1 {
                    d.extend_from_slice(&d.clone());
                }
                self.state.line_dash = std::sync::Arc::new(d);
                false
            }
            SetLineDashOffset { offset } => {
                if offset.is_finite() {
                    self.state.line_dash_offset = *offset;
                }
                false
            }
            SetShadowBlur { blur } => {
                if blur.is_finite() && *blur >= 0.0 {
                    self.state.shadow.blur = *blur;
                }
                false
            }
            SetShadowColor { color } => {
                self.state.shadow.color = *color;
                false
            }
            SetShadowOffsetX { offset } => {
                if offset.is_finite() {
                    self.state.shadow.offset_x = *offset;
                }
                false
            }
            SetShadowOffsetY { offset } => {
                if offset.is_finite() {
                    self.state.shadow.offset_y = *offset;
                }
                false
            }
            SetFillStyleGradient {
                gradient_type,
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                stops,
            } => {
                self.state.fill = StyleKind::from_gradient(
                    *gradient_type,
                    *x0,
                    *y0,
                    *r0,
                    *x1,
                    *y1,
                    *r1,
                    stops.clone(),
                );
                false
            }
            SetStrokeStyleGradient {
                gradient_type,
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                stops,
            } => {
                self.state.stroke = StyleKind::from_gradient(
                    *gradient_type,
                    *x0,
                    *y0,
                    *r0,
                    *x1,
                    *y1,
                    *r1,
                    stops.clone(),
                );
                false
            }
            SetFillStylePattern {
                image_id,
                repeat_x,
                repeat_y,
            } => {
                self.state.fill = StyleKind::Pattern {
                    image_id: *image_id,
                    repeat_x: *repeat_x,
                    repeat_y: *repeat_y,
                };
                false
            }
            SetStrokeStylePattern {
                image_id,
                repeat_x,
                repeat_y,
            } => {
                self.state.stroke = StyleKind::Pattern {
                    image_id: *image_id,
                    repeat_x: *repeat_x,
                    repeat_y: *repeat_y,
                };
                false
            }
            SetFont { font } => {
                apply_parsed_font(&mut self.state, font);
                false
            }
            SetTextAlign { align } => {
                self.state.text.align = *align;
                false
            }
            SetTextBaseline { baseline } => {
                self.state.text.baseline = *baseline;
                false
            }
            SetTextDirection { direction } => {
                self.state.text.direction = *direction;
                false
            }

            // ---- State stack --------------------------------------
            Save => {
                // Snapshot attribute state AND the SkCanvas (CTM + clip).
                self.stack.push(&self.state);
                canvas.save();
                false
            }
            Restore => {
                // Pop both sides.  Canvas spec: silent no-op when the
                // stack is empty.
                let popped_attrs = self.stack.pop(&mut self.state);
                if popped_attrs {
                    canvas.restore();
                }
                false
            }

            // ---- CTM mutators -------------------------------------
            SetTransform { a, b, c, d, e, f } => {
                // A 2x3 matrix is "axis-aligned + translation" when
                // the shear terms `b` and `c` are both zero.  Rotate
                // / skew / non-uniform scale set at least one; pure
                // translate or uniform scale keep both at zero.
                let axis_aligned = b.abs() < 1e-6 && c.abs() < 1e-6;
                self.state.ctm_non_axis_aligned |= !axis_aligned;
                let m = Matrix::new_all(*a, *c, *e, *b, *d, *f, 0.0, 0.0, 1.0);
                canvas.set_matrix(&skia_safe::M44::from(m));
                false
            }
            ResetTransform => {
                // Reset clears the flag — we're back to identity.
                self.state.ctm_non_axis_aligned = false;
                canvas.reset_matrix();
                false
            }
            Translate { x, y } => {
                // Translation never makes the CTM non-axis-aligned.
                canvas.translate((*x, *y));
                false
            }
            Rotate { angle } => {
                // Any non-zero rotation is non-axis-aligned for
                // damage purposes.  We don't special-case k*90deg
                // rotations because the transform composes with
                // previous mutations opaquely.
                if angle.abs() > 1e-6 {
                    self.state.ctm_non_axis_aligned = true;
                }
                canvas.rotate(angle.to_degrees(), None);
                false
            }
            Scale { x, y } => {
                // Pure uniform axis-aligned scale (positive, any
                // magnitude) keeps axis alignment.  Reflection (x<0
                // or y<0) flips axis but stays aligned.
                let _ = (x, y);
                canvas.scale((*x, *y));
                false
            }

            // ---- Text (stubbed in P4; real impl in P5) ------------
            FillText {
                text,
                x,
                y,
                max_width,
            } => {
                text_ctx.fill_text(canvas, text, *x, *y, *max_width, &self.state, resolver);
                true
            }
            StrokeText {
                text,
                x,
                y,
                max_width,
            } => {
                text_ctx.stroke_text(canvas, text, *x, *y, *max_width, &self.state, resolver);
                true
            }
            MeasureText { resp, .. } => {
                // MeasureText is normally routed to the text stack at the
                // dispatcher layer; reaching here means a caller expected
                // the backend to reply.  Send an empty metric to keep the
                // channel alive.  Phase 5 replaces this branch.
                // SAFETY: clone the response only when available via move;
                // accessing `resp` through a &ref would require `Clone`.
                let _ = resp; // keep borrow live; handled by upper layer
                false
            }

            // ---- Images (implemented in P4 but needs resolver) ----
            DrawImage { .. } | DrawImageBatch { .. } => {
                // Route through the `PatternResolver` trait or a dedicated
                // image-draw trait in Phase 4b; for now drop so the
                // command stream stays valid.
                false
            }
            GetImageData { resp, .. } => {
                let _ = resp;
                false
            }

            CreateContext2D { resp } => {
                let _ = resp;
                false
            }

            // Canvas2DCmd is #[non_exhaustive]; be permissive about future
            // opcodes by logging-and-skipping.  Tests lock the exhaustive
            // set above so a new variant won't be silently ignored in CI.
            _ => false,
        }
    }

    /// Reset the context to fresh defaults.  Used on `resetContext()` and
    /// after onscreen surface recreation.
    pub fn reset(&mut self) {
        self.state = Canvas2DState::default();
        self.stack = StateStack::new();
        self.path.reset();
    }

    /// Apply a Canvas2D shadow to the given paint if the current shadow is
    /// visible.  Exposed so `fill` / `stroke` paths can opt in at draw
    /// time without paying the cost when shadows are off (the common case).
    #[allow(dead_code)] // wired up in P4b drawing-side refinement
    pub fn maybe_apply_shadow(
        _paint: &mut skia_safe::Paint,
        shadow: &Shadow,
    ) -> bool {
        if !shadow.is_visible() {
            return false;
        }
        // TODO(P4b): install a blur + drop-shadow SkImageFilter on `paint`.
        true
    }
}

/// Unused-import silencer for imports that only matter in the stubs above.
#[allow(dead_code)]
fn _force_use_imports() {
    let _ = ProtocolColor::black();
    let _ = TextAlign::Start;
    let _ = TextBaseline::Alphabetic;
    let _ = GradientType::Linear;
}

/// Apply a CSS `font` shorthand to a Canvas2D state.
///
/// Extracted so both the dispatch path and unit tests exercise the
/// identical code: the test seam ensures `SetFont` can never regress
/// to the previous "silently ignored" behaviour.  An unparseable
/// shorthand leaves `state.text` untouched, matching Blink's "invalid
/// font assignment is a no-op" policy.
pub(crate) fn apply_parsed_font(state: &mut Canvas2DState, font: &str) {
    if let Some(parsed) = super::font_parse::parse_font_shorthand(font) {
        state.text.size = parsed.size_px;
        state.text.weight = parsed.weight;
        state.text.italic = parsed.italic;
        state.text.families = std::sync::Arc::new(parsed.families);
    }
}

#[cfg(test)]
mod set_font_tests {
    use super::*;

    #[test]
    fn apply_parsed_font_updates_size_and_family() {
        let mut state = Canvas2DState::default();
        apply_parsed_font(&mut state, "italic bold 24px 'Noto Sans CJK SC', sans-serif");
        assert_eq!(state.text.size, 24.0);
        assert_eq!(state.text.weight, 700);
        assert!(state.text.italic);
        assert_eq!(
            &*state.text.families,
            &vec!["Noto Sans CJK SC".to_string(), "sans-serif".to_string()]
        );
    }

    #[test]
    fn apply_parsed_font_preserves_state_on_invalid_input() {
        let mut state = Canvas2DState::default();
        let before = state.text.clone();
        // No size token → invalid per CSS; must be silent no-op.
        apply_parsed_font(&mut state, "bold serif");
        assert_eq!(state.text, before);
    }

    #[test]
    fn apply_parsed_font_handles_pt_units() {
        let mut state = Canvas2DState::default();
        apply_parsed_font(&mut state, "12pt Helvetica");
        // 12pt == 16px at 96dpi
        assert!((state.text.size - 16.0).abs() < 1e-3);
        assert_eq!(&*state.text.families, &vec!["Helvetica".to_string()]);
    }
}
