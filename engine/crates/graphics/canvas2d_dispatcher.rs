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

/// Conservative damage classifier.
///
/// The femtovg-era dispatcher inspected CTM / shadow / clip state to
/// produce a tight `OnscreenRect` damage — shrinking the area
/// eglSetDamageRegionKHR declares to the compositor.  That optimisation
/// is re-added in Phase 8 once the Skia state snapshot can be read
/// cheaply from the render thread; for now every draw produces
/// full-surface damage, which is correct-but-unoptimal.
#[inline]
pub(crate) fn classify_draw_damage(
    cmd: &Canvas2DCmd,
    _state: &crate::backend::gl::state::Canvas2DState,
) -> DamageEffect {
    use Canvas2DCmd::*;
    // Pure state / path-building commands never modify the framebuffer.
    // Everything that *might* draw pessimistically claims the whole
    // surface until the Phase-8 tight-bounds pass lands.
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
        | Save | Restore | SetTransform { .. } | ResetTransform
        | Translate { .. } | Rotate { .. } | Scale { .. }
        | Clip
        | MeasureText { .. } | GetImageData { .. }
        | CreateContext2D { .. } => DamageEffect::NoDamage,
        _ => DamageEffect::FullSurface,
    }
}

/// Legacy "partial-damage is safe" predicate; see
/// [`classify_draw_damage`].  Always `false` for now (i.e. we never let
/// the render thread use its precomputed dirty-rect to shrink scissor —
/// full-surface coverage is always correct).
#[inline]
pub(crate) fn state_allows_partial(_state: &crate::backend::gl::state::Canvas2DState) -> bool {
    false
}
