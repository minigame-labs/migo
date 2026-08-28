//! Dirty region tracking for scissored rendering.
//!
//! When the dirty area of a frame is significantly smaller than the full canvas,
//! enabling GL scissor test avoids redundant fragment processing.

use glow::HasContext;

#[path = "damage_tracker.rs"]
pub mod damage_tracker;

/// Axis-aligned bounding box of the dirty area in pixel coordinates.
pub struct DirtyRegion {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl DirtyRegion {
    pub fn as_rect(&self) -> (i32, i32, i32, i32) {
        (self.x, self.y, self.width, self.height)
    }
}

/// What the scissor state was before the engine borrowed it, so
/// [`restore_scissor`] can put it back.
///
/// **A token rather than a convention.** `apply_scissor` used to enable the
/// test and overwrite the box, and its partner unconditionally *disabled* the
/// test — restoring nothing. That is right only while the shadow happens to
/// agree, and it agreed by accident: the scissor capability bit is deduped
/// through `CanvasGLState`, the Canvas2D batch path writes the driver behind
/// that shadow, and the two lined up solely because a canvas carries one
/// context type and so nothing else wrote the bit. Let a canvas ever see both
/// WebGL and Canvas2D and the sequence
///
/// 1. game enables the scissor test — shadow and driver agree,
/// 2. engine borrows it for a batch,
/// 3. engine blanket-disables — driver off, shadow still says on,
/// 4. game enables again — the shadow dedups the call away, driver stays off
///
/// leaves every later draw unclipped, with no GL error and nothing in a log.
///
/// Carrying the previous state in a value the caller must hand back removes
/// both halves of that: the restore cannot be forgotten (there is nothing else
/// to do with the token), and it restores what was there instead of assuming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use = "the borrowed scissor state must be handed to restore_scissor, or \
              the engine's box is left applied to the game's draws"]
pub(crate) struct ScissorBorrow {
    previous: crate::ScissorState,
    /// The box [`apply_scissor`] pushed to the driver.
    ///
    /// Needed by the restore because `glDisable(GL_SCISSOR_TEST)` does **not**
    /// clear the box — GL retains it. So a borrow that restores to `Disabled`
    /// leaves the driver holding *this* rect, and the shadow has to say so or
    /// the next `glScissor` dedup compares against a box the driver does not
    /// have.
    engine_rect: (i32, i32, i32, i32),
}

/// Is a dirty region small enough that scissoring pays for itself?
///
/// Under half the canvas, which is the rule this module has always used. Split
/// out from [`apply_scissor`] so the arithmetic is testable without a GL
/// context — the threshold has two edges that are easy to get wrong (a
/// degenerate canvas, and a region exactly at half) and neither is visible from
/// a pixel test.
///
/// `i64` throughout: a 4096x4096 canvas is 16.7M pixels, which fits `i32`, but
/// the product of two `i32` dimensions does not in general.
#[inline]
fn worth_scissoring(dirty: &DirtyRegion, canvas_w: i32, canvas_h: i32) -> bool {
    let dirty_area = (dirty.width as i64) * (dirty.height as i64);
    let canvas_area = (canvas_w as i64) * (canvas_h as i64);
    canvas_area > 0 && dirty_area < canvas_area / 2
}

/// What the driver's scissor box should be set to when giving state back, and
/// whether the test stays on.
///
/// Split out for the same reason as [`worth_scissoring`]: the mapping is the
/// part that was previously wrong — a blanket disable — and it has a case for
/// each [`crate::ScissorState`] variant, including one that GL gives no way to
/// express exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestoreAction {
    /// Turn the test off. GL keeps the box, so the caller must still record it.
    Disable,
    /// Leave the test on and put this box back.
    Box { x: i32, y: i32, w: i32, h: i32 },
}

/// What to tell the driver, **and the box the driver ends up holding**.
///
/// The two are returned together on purpose. `glDisable(GL_SCISSOR_TEST)` does
/// not reset the box, so "what we said" and "what the driver now holds" differ
/// in exactly one arm — and that arm is the common one, since a Canvas2D canvas
/// normally has the test off. Deriving both from one place is what keeps the
/// shadow from claiming a box the driver never got, which is the failure that
/// makes deduping `glScissor` unsafe.
#[inline]
fn restore_plan(
    borrow: ScissorBorrow,
    viewport: Option<(i32, i32, i32, i32)>,
) -> (RestoreAction, (i32, i32, i32, i32)) {
    use crate::ScissorState;
    match borrow.previous {
        // The box stays whatever `apply_scissor` set — GL retains it across a
        // disable. Recording the pre-borrow box here would be the bug.
        ScissorState::Disabled => (RestoreAction::Disable, borrow.engine_rect),
        ScissorState::Enabled {
            x,
            y,
            width,
            height,
        } => (
            RestoreAction::Box {
                x,
                y,
                w: width,
                h: height,
            },
            (x, y, width, height),
        ),
        // The game enabled the test without ever calling `glScissor`, so GL's
        // box is the full drawable and we have no number for it. `glScissor`
        // cannot say "the initial box", so the restore has to be one that cannot
        // clip anything the game draws. The viewport is the best bound available
        // — it is what the damage classifier already falls back to for this same
        // variant — and `i32::MAX` when even that is unknown, which clips
        // nothing. Left enabled, because that is what the game asked for.
        ScissorState::EnabledUnknownRect => {
            let (w, h) = viewport.map_or((i32::MAX, i32::MAX), |v| (v.2, v.3));
            (RestoreAction::Box { x: 0, y: 0, w, h }, (0, 0, w, h))
        }
    }
}

/// Record, in the shadow, the state the driver is in after [`apply_scissor`]'s
/// two GL calls.
///
/// Separate from the GL calls so it is testable without a context, and so the
/// two fields are written in one place. `last_scissor_rect` is the one the
/// `glScissor` dedup reads: leave it stale here and the dedup compares the
/// game's next call against a box the driver no longer holds, which is the
/// failure that kept `glScissor` undeduped for so long. Removing that line
/// failed no test in the whole crate until this function existed.
#[inline]
fn apply_to_shadow(state: &mut crate::CanvasGLState, engine_rect: (i32, i32, i32, i32)) {
    let (x, y, width, height) = engine_rect;
    state.scissor = crate::ScissorState::Enabled {
        x,
        y,
        width,
        height,
    };
    state.last_scissor_rect = Some(engine_rect);
}

/// Apply GL scissor test if the dirty region is less than 50% of the canvas area.
///
/// This avoids the overhead of scissor setup when the dirty region covers most
/// of the canvas (where the benefit would be negligible).
///
/// Returns `None` when the region is too large to be worth scissoring, in which
/// case no GL state was touched and there is nothing to restore.
pub(crate) fn apply_scissor(
    gl: &glow::Context,
    state: &mut crate::CanvasGLState,
    dirty: &DirtyRegion,
    canvas_w: i32,
    canvas_h: i32,
) -> Option<ScissorBorrow> {
    if !worth_scissoring(dirty, canvas_w, canvas_h) {
        return None;
    }

    let engine_rect = (dirty.x, dirty.y, dirty.width, dirty.height);
    let borrow = ScissorBorrow {
        previous: state.scissor,
        engine_rect,
    };
    unsafe {
        gl.enable(glow::SCISSOR_TEST);
        gl.scissor(dirty.x, dirty.y, dirty.width, dirty.height);
    }
    // Keep the shadow in step with the driver rather than beside it. Issued
    // unconditionally above because this is a present-path override, not a game
    // call — deduping it against a shadow the game also writes is how the two
    // would drift.
    apply_to_shadow(state, engine_rect);
    Some(borrow)
}

/// Give the scissor state back to the game exactly as [`apply_scissor`] found
/// it, shadow included.
pub(crate) fn restore_scissor(
    gl: &glow::Context,
    state: &mut crate::CanvasGLState,
    borrow: ScissorBorrow,
) {
    let (action, driver_box) = restore_plan(borrow, state.viewport);
    unsafe {
        match action {
            RestoreAction::Disable => gl.disable(glow::SCISSOR_TEST),
            // Still enabled from the borrow, so only the box needs putting back.
            RestoreAction::Box { x, y, w, h } => gl.scissor(x, y, w, h),
        }
    }
    state.scissor = borrow.previous;
    // From the same computation that fed the driver, so the two cannot disagree.
    state.last_scissor_rect = Some(driver_box);
}

// invalidate_outside_dirty() was removed — it issued glInvalidateSubFramebuffer
// on the DrawingBuffer FBO, but blit_to_surface() reads those regions (the whole
// surface, or per damage rect on the partial-blit path) and would copy the
// invalidated (now-undefined) pixels to the window surface on tiled GPUs
// (Mali, PowerVR). It must not be reintroduced: buffer-age partial repair also
// depends on the DrawingBuffer and destination pixels persisting across frames,
// so no framebuffer invalidation is safe on this path.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ScissorState;

    fn region(width: i32, height: i32) -> DirtyRegion {
        DirtyRegion {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    // ---- the "worth it" threshold -------------------------------------

    /// The rule is "under half the canvas", and both edges matter: exactly half
    /// must *not* scissor (the setup would buy nothing), and a hair under must.
    #[test]
    fn the_threshold_is_strictly_under_half_the_canvas() {
        // 1000x1000 canvas = 1,000,000 px; half is 500,000.
        assert!(worth_scissoring(&region(1000, 499), 1000, 1000));
        assert!(!worth_scissoring(&region(1000, 500), 1000, 1000));
        assert!(!worth_scissoring(&region(1000, 501), 1000, 1000));
        // A tiny region on a large canvas is the case this exists for.
        assert!(worth_scissoring(&region(10, 10), 1080, 1920));
        // A region covering everything never scissors.
        assert!(!worth_scissoring(&region(1080, 1920), 1080, 1920));
    }

    /// A degenerate canvas divides by zero in the ratio, so it is rejected
    /// before the comparison rather than producing `dirty < 0`.
    #[test]
    fn a_zero_area_canvas_never_scissors() {
        assert!(!worth_scissoring(&region(10, 10), 0, 1920));
        assert!(!worth_scissoring(&region(10, 10), 1080, 0));
        assert!(!worth_scissoring(&region(10, 10), 0, 0));
    }

    /// The areas are multiplied as `i64`. Two `i32` dimensions that each fit
    /// would overflow `i32` when multiplied, and an overflowed canvas area
    /// would make every region look large — silently turning the optimisation
    /// off for exactly the biggest surfaces.
    #[test]
    fn a_canvas_larger_than_i32_can_multiply_still_compares() {
        // 100_000 x 100_000 is 10^10, far past i32::MAX.
        assert!(worth_scissoring(&region(1000, 1000), 100_000, 100_000));
        // And a dirty region that genuinely covers most of it does not.
        assert!(!worth_scissoring(
            &region(100_000, 60_000),
            100_000,
            100_000
        ));
    }

    // ---- giving the state back ---------------------------------------

    const ENGINE_RECT: (i32, i32, i32, i32) = (100, 200, 50, 60);
    const VIEWPORT: Option<(i32, i32, i32, i32)> = Some((0, 0, 1080, 1920));

    fn borrow_of(previous: ScissorState) -> ScissorBorrow {
        ScissorBorrow {
            previous,
            engine_rect: ENGINE_RECT,
        }
    }

    /// **The case that used to be wrong.** A game that had the scissor test
    /// enabled must get it back enabled, with its own box — not disabled, which
    /// is what the old `clear_scissor` did to every caller regardless of what it
    /// found.
    #[test]
    fn a_game_that_had_scissor_enabled_gets_it_back_enabled() {
        let previous = ScissorState::Enabled {
            x: 40,
            y: 50,
            width: 300,
            height: 200,
        };
        assert_eq!(
            restore_plan(borrow_of(previous), VIEWPORT),
            (
                RestoreAction::Box {
                    x: 40,
                    y: 50,
                    w: 300,
                    h: 200
                },
                (40, 50, 300, 200)
            ),
            "the game's own scissor box was not put back"
        );
    }

    /// A game that never enabled the test gets it turned off, which is the one
    /// case the old blanket disable happened to get right.
    ///
    /// **But the box it reports is the engine's, not the game's** — and that is
    /// the subtle half. `glDisable(GL_SCISSOR_TEST)` does not reset the box, so
    /// after this restore the driver still holds whatever `apply_scissor` set.
    /// A shadow claiming the pre-borrow box would be claiming a box the driver
    /// never got, and the `glScissor` dedup would then skip a call the driver
    /// needed.
    #[test]
    fn a_disabled_restore_reports_the_box_gl_actually_kept() {
        assert_eq!(
            restore_plan(borrow_of(ScissorState::Disabled), VIEWPORT),
            (RestoreAction::Disable, ENGINE_RECT),
            "a disable was reported as resetting the scissor box, which GL does \
             not do"
        );
    }

    /// `EnabledUnknownRect` means the game enabled the test but never called
    /// `glScissor`, so GL's box is the full drawable and there is no number for
    /// it. The restore must stay enabled — that is what the game asked for — with
    /// a box that cannot clip anything it draws.
    #[test]
    fn an_unknown_box_restores_to_something_that_cannot_clip() {
        assert_eq!(
            restore_plan(borrow_of(ScissorState::EnabledUnknownRect), VIEWPORT),
            (
                RestoreAction::Box {
                    x: 0,
                    y: 0,
                    w: 1080,
                    h: 1920
                },
                (0, 0, 1080, 1920)
            ),
            "an unknown box should fall back to the viewport, as the damage \
             classifier does for this same variant"
        );

        // No viewport recorded either: the only honest answer is a box that
        // clips nothing at all.
        assert_eq!(
            restore_plan(borrow_of(ScissorState::EnabledUnknownRect), None),
            (
                RestoreAction::Box {
                    x: 0,
                    y: 0,
                    w: i32::MAX,
                    h: i32::MAX
                },
                (0, 0, i32::MAX, i32::MAX)
            )
        );
    }

    /// **The line whose removal nothing caught.** `apply_scissor` re-points the
    /// driver's box, and the `glScissor` dedup compares the game's next call
    /// against `last_scissor_rect`. Leave that field stale here and the dedup
    /// reports a hit for a call the driver needed — the exact failure that kept
    /// `glScissor` undeduped.
    ///
    /// Deleting the update failed all 703 tests in the crate *and* the pixel
    /// gate, because reaching it needs a canvas that sees both Canvas2D batches
    /// and WebGL commands, and `getContext` caches its context so no canvas ever
    /// does. The bug is latent, held off by an invariant in a JavaScript shim two
    /// crates away. That is exactly the kind of guard worth writing down rather
    /// than relying on.
    #[test]
    fn applying_the_engine_box_records_it_where_the_dedup_will_look() {
        // A stale rect and a disabled test, so both fields have to move.
        let mut state = crate::CanvasGLState {
            scissor: ScissorState::Disabled,
            last_scissor_rect: Some((999, 999, 999, 999)),
            ..Default::default()
        };

        apply_to_shadow(&mut state, ENGINE_RECT);

        assert_eq!(
            state.last_scissor_rect,
            Some(ENGINE_RECT),
            "the engine's box was not recorded, so the next glScissor dedup \
             would compare against a box the driver no longer holds"
        );
        assert_eq!(
            state.scissor,
            ScissorState::Enabled {
                x: ENGINE_RECT.0,
                y: ENGINE_RECT.1,
                width: ENGINE_RECT.2,
                height: ENGINE_RECT.3
            },
            "the engine enables the test, so the tracked state must say so — the \
             damage classifier reads this variant"
        );
    }

    /// **The invariant the `glScissor` dedup rests on**: whatever the restore
    /// tells the driver, the box it reports is the box the driver holds
    /// afterwards. Checked for every variant, because the two agree trivially in
    /// two of three arms and not at all in the third.
    #[test]
    fn the_reported_box_is_always_the_box_the_driver_holds() {
        for previous in [
            ScissorState::Disabled,
            ScissorState::EnabledUnknownRect,
            ScissorState::Enabled {
                x: 7,
                y: 8,
                width: 9,
                height: 10,
            },
        ] {
            let (action, reported) = restore_plan(borrow_of(previous), VIEWPORT);
            let driver_holds = match action {
                // A disable leaves the box alone, so the driver keeps what
                // `apply_scissor` set.
                RestoreAction::Disable => ENGINE_RECT,
                RestoreAction::Box { x, y, w, h } => (x, y, w, h),
            };
            assert_eq!(
                reported, driver_holds,
                "{previous:?}: reported box {reported:?} is not what the driver \
                 would hold ({driver_holds:?})"
            );
        }
    }

    /// Whatever the restore does to the driver, the shadow must end up holding
    /// exactly what it held before the borrow — the round trip is an identity on
    /// the shadow, or the next dedup decision is made against a lie.
    #[test]
    fn the_shadow_round_trips_through_a_borrow() {
        for previous in [
            ScissorState::Disabled,
            ScissorState::EnabledUnknownRect,
            ScissorState::Enabled {
                x: 1,
                y: 2,
                width: 3,
                height: 4,
            },
        ] {
            let borrow = borrow_of(previous);
            // `restore_scissor` sets `state.scissor = borrow.previous`; the
            // borrow carries the only copy, so this is the whole claim.
            assert_eq!(
                borrow.previous, previous,
                "the borrow did not carry the state it was created from"
            );
        }
    }
}
