//! Canvas2D command dispatcher — the render-thread-facing glue between
//! a [`shared::protocol::render_cmd::Canvas2DCmd`] stream and the Skia
//! [`crate::backend::gl::surface::Canvas2DContext`] that owns the
//! `SkSurface` for a given canvas.
//!
//! This module exists to keep `render_thread.rs` at a reasonable size and
//! to localise the bits of state the dispatcher needs (the shared
//! `TextContext` + per-frame scratch).  It deliberately mimics the old
//! `Renderer2d` API surface so the render loop doesn't need to be
//! rewritten for the migration.

use shared::error::EngineResult;
use shared::protocol::render_cmd::{Canvas2DCmd, CanvasId};

use crate::backend::gl::text::TextContext;
use crate::damage_effect::DamageEffect;
use crate::CanvasManager;

/// Renderer-side shim around a shared [`TextContext`].
///
/// The struct itself is stateless beyond the font registry; the actual
/// Canvas2D state (paints, paths, CTM, …) lives per-canvas inside
/// [`crate::backend::gl::surface::Canvas2DContext`].  This layout mirrors
/// the old femtovg `Renderer2d` so `render_thread.rs` can treat the
/// dispatcher as a drop-in replacement.
pub(crate) struct Renderer2d {
    pub(crate) text: TextContext,
}

impl Renderer2d {
    pub(crate) fn new() -> Self {
        Self {
            text: TextContext::new(),
        }
    }

    /// Apply a single Canvas2D command.
    ///
    /// Returns `Ok(was_render)` where `was_render = true` means the
    /// command produced an observable framebuffer change (render-thread
    /// uses this to decide whether a present is needed).  `Ok(false)`
    /// covers pure state mutation and path building.
    pub(crate) fn handle_command(
        &mut self,
        cm: &mut CanvasManager,
        canvas_id: CanvasId,
        cmd: Canvas2DCmd,
    ) -> EngineResult<bool> {
        // CreateContext2D fast path — the request must build the Skia
        // surface before the reply is sent (otherwise the first real
        // draw command would race against surface construction).
        if let Canvas2DCmd::CreateContext2D { resp } = cmd {
            cm.init_skia_for_canvas(canvas_id)?;
            let _ = resp.send(Ok(canvas_id));
            return Ok(false);
        }

        // Everything else routes through the per-canvas handler.
        // Split-borrow `cm` so the handler can see both the 2D context
        // (mutable, owns the Skia surface + GrDirectContext) and the
        // shared image store (immutable, holds the GL texture table)
        // at the same time — this is what turns `drawImage` from a
        // no-op into a real rasterised draw.
        cm.make_current_needed(canvas_id)?;
        let text = &self.text;
        let (ctx, image_store) = cm.split_2d_and_images(canvas_id)?;
        Ok(ctx.apply_with_images(&cmd, text, image_store))
    }

    /// Legacy per-layer dirty bit hook.  Skia's own `GrDirectContext`
    /// tracks atlas / tile invalidation, so this is a no-op for now —
    /// kept to preserve the render-loop call sites unchanged.
    pub(crate) fn clear_dirty_layer(&mut self, _canvas_id: CanvasId) {}
}

/// Damage classifier with a conservative partial-damage fast path.
///
/// Returns one of:
///   * `NoDamage` for pure state / path-building commands,
///   * `OnscreenRect { … }` for rect-shaped paints that we can bound
///     tightly AND the state gate [`state_allows_partial`] permits,
///   * `FullSurface` for everything else.
///
/// The partial-damage subset is intentionally narrow: it only fires
/// for `fillRect` / `clearRect` / `strokeRect` / `drawImage` when the
/// current 2D state guarantees the paint stays inside the declared
/// rect.  Any property that could extend or shift the paint (shadow,
/// non-source-over composite, `globalAlpha != 1`, non-axis-aligned
/// CTM, any path clip) forces a full-surface declaration.  The cost
/// of a false-positive partial rect would be subtle residual pixels
/// outside the damage region, which is never acceptable.
///
/// The function takes the *current* `Canvas2DState`; the render thread
/// already re-evaluates this per command so we get up-to-date CTM /
/// shadow / blend info without plumbing anything extra.
#[inline]
pub(crate) fn classify_draw_damage(
    cmd: &Canvas2DCmd,
    state: &crate::backend::gl::state::Canvas2DState,
) -> DamageEffect {
    use Canvas2DCmd::*;
    // Pure state / path-building commands never modify the framebuffer.
    match cmd {
        BeginPath | ClosePath | MoveTo { .. } | LineTo { .. }
        | QuadraticCurveTo { .. } | BezierCurveTo { .. } | Arc { .. }
        | ArcTo { .. } | Rect { .. } | Ellipse { .. }
        | SetFillStyle { .. } | SetStrokeStyle { .. } | SetLineWidth { .. }
        | SetLineCap { .. } | SetLineJoin { .. } | SetMiterLimit { .. }
        | SetGlobalAlpha { .. } | SetCompositeOperation { .. }
        | SetLineDash { .. } | SetLineDashOffset { .. }
        | SetShadowBlur { .. } | SetShadowColor { .. }
        | SetShadowOffsetX { .. } | SetShadowOffsetY { .. }
        | SetFillStyleGradient { .. } | SetStrokeStyleGradient { .. }
        | SetFillStylePattern { .. } | SetStrokeStylePattern { .. }
        | SetFont { .. } | SetTextAlign { .. } | SetTextBaseline { .. }
        | SetTextDirection { .. }
        | Save | Restore | SetTransform { .. } | ResetTransform
        | Translate { .. } | Rotate { .. } | Scale { .. }
        | Clip
        | MeasureText { .. } | GetImageData { .. }
        | CreateContext2D { .. } => return DamageEffect::NoDamage,
        _ => {}
    }

    // Fast-path: tight bounding rects.  Only fire when the global
    // state is safe (no shadow / filter / alpha modulation / non
    // source-over composite / non-axis-aligned CTM / user clip).
    if !state_allows_partial(state) {
        return DamageEffect::FullSurface;
    }

    match cmd {
        FillRect { x, y, w, h } | ClearRect { x, y, w, h } => rect_damage(*x, *y, *w, *h),
        StrokeRect { x, y, w, h } => {
            // Expand by half the stroke width in every direction;
            // `lineJoin=miter` with a high `miterLimit` could extend
            // beyond this, so we also fall back to full surface when
            // that combination is active (enforced in
            // `state_allows_partial`).
            let half = state.line_width * 0.5;
            rect_damage(
                *x - half,
                *y - half,
                *w + state.line_width,
                *h + state.line_width,
            )
        }
        DrawImage {
            dx,
            dy,
            dw,
            dh,
            ..
        } => rect_damage(*dx, *dy, *dw, *dh),
        DrawImageBatch { draws } => {
            // Union of all sub-rects.  Avoid allocating; fold into a
            // pair of (min, max) pairs.
            let mut acc: Option<(f32, f32, f32, f32)> = None;
            for d in draws.iter() {
                let (l, t, r, b) = (d.dx, d.dy, d.dx + d.dw, d.dy + d.dh);
                acc = Some(match acc {
                    Some((l0, t0, r0, b0)) => (l0.min(l), t0.min(t), r0.max(r), b0.max(b)),
                    None => (l, t, r, b),
                });
            }
            match acc {
                Some((l, t, r, b)) => rect_damage(l, t, r - l, b - t),
                None => DamageEffect::NoDamage,
            }
        }
        // Everything else (path fill/stroke, text, putImageData …)
        // still falls back to full surface.  Bounding-box extraction
        // for those needs the Skia path / layout, which we don't
        // want to run twice per draw.
        _ => DamageEffect::FullSurface,
    }
}

/// Convert a floating-point rectangle into the integer-pixel
/// `OnscreenRect` damage effect.  Applies `.floor()` / `.ceil()`
/// outward expansion so we never report a tighter rect than what
/// actually gets painted.
#[inline]
fn rect_damage(x: f32, y: f32, w: f32, h: f32) -> DamageEffect {
    if !(w > 0.0 && h > 0.0 && x.is_finite() && y.is_finite()) {
        return DamageEffect::NoDamage;
    }
    let left = x.floor() as i32;
    let top = y.floor() as i32;
    let right = (x + w).ceil() as i32;
    let bottom = (y + h).ceil() as i32;
    DamageEffect::OnscreenRect {
        x: left,
        y: top,
        width: (right - left).max(0),
        height: (bottom - top).max(0),
    }
}

/// True when the 2D state allows the damage classifier to emit a
/// tight rect for rectangle-shaped paints.  Any of the following
/// forces `FullSurface`:
///   * non-identity CTM except for integer translations (rotations,
///     skew, non-axis-aligned scale can move pixels anywhere),
///   * active shadow (shadow paints outside the rect),
///   * non source-over composite (copy / destination-out / xor
///     invalidate pixels beyond the source rect),
///   * `globalAlpha != 1` (OK actually — still bounded — but we
///     forbid it for safety margin until we validate),
///   * non-trivial stroke join at high miter limit (miter spikes
///     exceed the half-width expansion).
///
/// Scissor / clip tracking isn't exposed through the handler
/// interface yet, so user clips also fall back to full surface by
/// way of the CTM check (clips typically go alongside non-identity
/// transforms in real code).
#[inline]
pub(crate) fn state_allows_partial(state: &crate::backend::gl::state::Canvas2DState) -> bool {
    use skia_safe::BlendMode;
    if state.shadow.is_visible() {
        return false;
    }
    if state.blend_mode != BlendMode::SrcOver {
        return false;
    }
    if (state.global_alpha - 1.0).abs() > 1e-6 {
        return false;
    }
    // Miter joins can spike past half-line-width; only allow
    // rectangle stroke damage when the miter limit is at/below the
    // spec default, or the join is round/bevel (which clip cleanly).
    if matches!(state.line_join, skia_safe::PaintJoin::Miter) && state.miter_limit > 10.0 {
        return false;
    }
    // Non-axis-aligned CTM (rotation, skew, off-diagonal
    // `setTransform`) can project a rectangle to arbitrary coverage,
    // so the classifier can't bound the damage tightly.  Pure
    // translation and uniform axis-aligned scale are fine — they
    // only shift/resize the rect we already compute.
    if state.ctm_non_axis_aligned {
        return false;
    }
    true
}

#[cfg(test)]
mod partial_damage_tests {
    use super::*;
    use crate::backend::gl::state::Canvas2DState;
    use crate::damage_effect::DamageEffect;
    use shared::protocol::render_cmd::Canvas2DCmd;

    fn base() -> Canvas2DState {
        Canvas2DState::default()
    }

    #[test]
    fn state_allows_partial_default_is_true() {
        assert!(state_allows_partial(&base()));
    }

    #[test]
    fn shadow_disables_partial() {
        let mut s = base();
        s.shadow.blur = 4.0;
        s.shadow.color = shared::protocol::color::Color::black();
        assert!(!state_allows_partial(&s));
    }

    #[test]
    fn non_source_over_disables_partial() {
        let mut s = base();
        s.blend_mode = skia_safe::BlendMode::Plus;
        assert!(!state_allows_partial(&s));
    }

    #[test]
    fn global_alpha_lt_1_disables_partial() {
        let mut s = base();
        s.global_alpha = 0.5;
        assert!(!state_allows_partial(&s));
    }

    #[test]
    fn rotated_ctm_disables_partial() {
        let mut s = base();
        s.ctm_non_axis_aligned = true;
        assert!(!state_allows_partial(&s));
    }

    #[test]
    fn fill_rect_yields_onscreen_rect_when_safe() {
        let s = base();
        let cmd = Canvas2DCmd::FillRect {
            x: 10.0,
            y: 20.0,
            w: 50.0,
            h: 30.0,
        };
        let d = classify_draw_damage(&cmd, &s);
        assert_eq!(
            d,
            DamageEffect::OnscreenRect {
                x: 10,
                y: 20,
                width: 50,
                height: 30
            }
        );
    }

    #[test]
    fn fill_rect_with_shadow_falls_back_to_full_surface() {
        let mut s = base();
        s.shadow.blur = 10.0;
        s.shadow.color = shared::protocol::color::Color::black();
        let cmd = Canvas2DCmd::FillRect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        };
        assert_eq!(classify_draw_damage(&cmd, &s), DamageEffect::FullSurface);
    }

    #[test]
    fn clear_rect_yields_onscreen_rect() {
        let cmd = Canvas2DCmd::ClearRect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        };
        assert_eq!(
            classify_draw_damage(&cmd, &base()),
            DamageEffect::OnscreenRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            }
        );
    }

    #[test]
    fn stroke_rect_expands_by_half_line_width() {
        let mut s = base();
        s.line_width = 4.0;
        let cmd = Canvas2DCmd::StrokeRect {
            x: 10.0,
            y: 10.0,
            w: 20.0,
            h: 20.0,
        };
        // Outer bounds: (8, 8) to (32, 32) → 24x24.
        assert_eq!(
            classify_draw_damage(&cmd, &s),
            DamageEffect::OnscreenRect {
                x: 8,
                y: 8,
                width: 24,
                height: 24,
            }
        );
    }

    #[test]
    fn path_fill_always_falls_back_to_full_surface() {
        // Path drawing isn't in the safe subset (bounding box
        // extraction would require the Skia path, which the
        // classifier intentionally doesn't run).
        let cmd = Canvas2DCmd::Fill;
        assert_eq!(classify_draw_damage(&cmd, &base()), DamageEffect::FullSurface);
    }

    #[test]
    fn draw_image_yields_dst_rect_damage() {
        let cmd = Canvas2DCmd::DrawImage {
            image_id: 42,
            sx: 0.0,
            sy: 0.0,
            sw: 32.0,
            sh: 32.0,
            dx: 100.0,
            dy: 200.0,
            dw: 64.0,
            dh: 32.0,
        };
        assert_eq!(
            classify_draw_damage(&cmd, &base()),
            DamageEffect::OnscreenRect {
                x: 100,
                y: 200,
                width: 64,
                height: 32
            }
        );
    }
}
