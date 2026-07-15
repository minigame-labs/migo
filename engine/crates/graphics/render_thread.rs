//! # Render thread driver
//!
//! Hosts the Migo render loop that drains [`RenderCommand`]s, runs
//! Canvas2D / WebGL / upload work, and drives the
//! `eglSwapBuffers` + RAF signal pipeline.
//!
//! ## Migration: command handlers → `RenderLoopState` methods
//!
//! The [`RenderThread::spawn`] body below has historically been
//! one ~700-line lambda with three nested closures passing 11
//! arguments each (`handle_one_cmd`, `drain_cmds`,
//! `present_frame_and_signal_raf`).  F-3 landed the
//! [`crate::render_loop::RenderLoopState`] struct that bundles
//! every mutable per-loop value; the migration to method
//! dispatch is happening incrementally — adding a new command
//! variant no longer has to touch every closure's parameter list.
//! The existing closures stay as-is until each is individually
//! converted, at which point the struct's `pub(crate)` fields
//! let the method body reach the sub-components without
//! refactoring the disjoint borrows the current closures rely on.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use crate::{
    CanvasHandler, CanvasManager, RendererGL,
    canvas2d_dispatcher::Renderer2d,
    damage_effect::DamageEffect,
    dirty_region,
    frame_scheduler::FrameScheduler,
    onscreen_window_from_surface,
    render_frame_state::{DEFERRED_CLEANUP_UNUSED_AGE, DeferredCleanupCadence},
    render_server::RenderServer,
    surface_system::SurfaceSystem,
};
use crossbeam_channel::{Receiver, select, tick};
use glow::HasContext;
use shared::command_vec_pool::{recycle_canvas_command_vec, recycle_gl_command_vec};
use shared::error::{EngineError, EngineResult, ErrorCode};
use shared::protocol::render_cmd::{CanvasBatchPayload, CanvasId, GlBatchPayload, RenderCommand};
use shared::render_command_sender::CommandSender;
use shared::render_event::{RenderEvent, RenderEventReceiver, RenderEventSender};
use shared::surface::SurfaceRef;
use shared::{FrameOp, FramePacket};
use tracing::{error, info, warn};

use shared::raf_signal::RafSender;

/// Threshold at which `raf_drop_streak` turns into a
/// `RenderEvent::RafBackpressure` emission (P2-10).  Three
/// consecutive drops is stricter than a single-frame jitter but
/// lax enough that an actually stalled producer trips it within a
/// 50 ms window at 60 Hz.
const RAF_BACKPRESSURE_STREAK_THRESHOLD: u32 = 3;

/// Report a context-recovery failure at most once per loss episode.
///
/// Recovery is retried every frame while the context is lost; emitting a
/// `ContextRecovered{success:false}` event, waking the host, and (downstream)
/// firing a Java `notify_error` on every failed retry would flood at frame
/// rate. The loss epoch bumps on each fresh loss, so gating on it reports the
/// first failure of an episode and then stays silent until recovery succeeds or
/// a new loss starts. Returns `true` only for that first report so callers can
/// gate warning logs on the same epoch and avoid a 60-120 logs/s failure loop.
fn report_recovery_failure(
    context_lost: &Arc<shared::op_state::ContextLostState>,
    events: &RenderEventSender,
    last_recovery_fail_epoch: &mut u64,
) -> bool {
    let (_, epoch) = context_lost.snapshot();
    if epoch == *last_recovery_fail_epoch {
        return false;
    }
    *last_recovery_fail_epoch = epoch;
    // `emit` on a wake-backed channel signals the host exactly once per
    // successful enqueue, so no separate manual nudge is needed here.
    events.emit(RenderEvent::ContextRecovered { success: false });
    true
}

pub struct RenderThread {
    cmd_tx: CommandSender,
    shutdown_requested: Arc<std::sync::atomic::AtomicBool>,
    /// Render-thread → host event feedback channel.  Exposed via
    /// [`Self::events`] so the host runtime can subscribe and
    /// forward events into JS or the debug overlay.  See
    /// [`shared::render_event`] for the policy.
    event_rx: RenderEventReceiver,
    /// F-2: type-erased measurer handle shared with the JS
    /// thread's `CanvasOpState`.  Cloned into the host runtime
    /// wiring layer via [`Self::text_measurer`]; subsequent
    /// `op_measure_text_flat` calls bypass the cross-thread
    /// channel entirely when the host has installed it.
    text_measurer: shared::text_measurer::SharedTextMeasurer,
    handle: Option<thread::JoinHandle<()>>,
}

struct VsyncFrameDecision {
    should_signal_raf: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    should_present: bool,
    raf_time_ms: f64,
}

fn canvas2d_batch_should_mark_present_dirty(
    canvas_id: u32,
    batch_dirty: bool,
    present: bool,
    has_dirty_rect: bool,
) -> bool {
    present && canvas_id == 1 && (batch_dirty || has_dirty_rect)
}

/// Intermediate result from evaluating whether scissor optimization applies.
struct BatchScissorSetup {
    region: dirty_region::DirtyRegion,
    canvas_w: i32,
    canvas_h: i32,
}

/// Evaluate whether a canvas batch warrants scissor optimization.
///
/// Returns `Some` with the computed dirty region (in GL bottom-left-origin
/// coordinates) and canvas dimensions when all preconditions are met:
/// `present` is true, a dirty rect is provided, and canvas size is known.
///
/// The caller is still responsible for ensuring the target canvas is the
/// current GL context before issuing any GL scissor calls.
fn prepare_batch_scissor(
    present: bool,
    dirty_rect: &Option<shared::protocol::render_cmd::DirtyRect>,
    canvas_size: Option<(u32, u32)>,
) -> Option<BatchScissorSetup> {
    if !present {
        return None;
    }
    let rect = dirty_rect.as_ref()?;
    let (cw, ch) = canvas_size?;
    let cw = cw as i32;
    let ch = ch as i32;
    Some(BatchScissorSetup {
        region: dirty_region::DirtyRegion {
            x: rect.x.floor() as i32,
            y: (ch - (rect.y + rect.height).ceil() as i32).max(0),
            width: rect.width.ceil() as i32,
            height: rect.height.ceil() as i32,
        },
        canvas_w: cw,
        canvas_h: ch,
    })
}

fn mark_surface_destroyed(surface: &mut SurfaceSystem) {
    surface.on_surface_destroyed();
}

fn execute_canvas_batch(
    cm: &mut CanvasManager,
    gl: &glow::Context,
    renderer_2d: &mut Renderer2d,
    payload: CanvasBatchPayload,
) -> bool {
    use crate::canvas2d_dispatcher::classify_draw_damage;
    use crate::damage_effect::DamageEffect;

    let canvas_id = payload.canvas_id;
    let mut commands = payload.commands;
    let present = payload.present;
    let mut batch_dirty = false;
    let is_onscreen = canvas_id == shared::protocol::render_cmd::CanvasId::from(1u32);

    // Lazy per-context Skia reset.  `skia_state_stale` is flipped
    // by `execute_gl_batch` whenever WebGL mutates GL state; on the
    // *next* Canvas2D batch for this canvas we issue exactly one
    // `GrDirectContext::reset()` and clear the flag.  Pure-Canvas2D
    // frames pay zero resets; cross-canvas WebGL→Canvas2D boundaries
    // only invalidate the affected context instead of the old
    // manager-global flag's broadcast invalidation.
    if cm.make_current_needed(canvas_id).is_ok() {
        if let Ok(ctx) = cm.get_2d_context_mut(canvas_id) {
            ctx.reset_gl_state_if_stale();
        }
    }

    // Layer 2 scissor trust gate: if Canvas2D state has non-identity
    // transform or active shadow, the JS-side dirty hint may underestimate
    // the actual draw area. Discard it to prevent scissor from clipping.
    let dirty_rect = {
        use crate::canvas2d_dispatcher::state_allows_partial;
        let state_safe = cm
            .get_2d_context_mut(canvas_id)
            .map(|ctx| state_allows_partial(&ctx.renderer.state))
            .unwrap_or(true); // no context yet → no draws → hint is fine
        if state_safe { payload.dirty_rect } else { None }
    };

    let scissor_applied = if let Some(setup) =
        prepare_batch_scissor(present, &dirty_rect, cm.get_canvas_size(canvas_id).ok())
    {
        // Bind the target canvas before touching GL state so scissor
        // hits the correct EGL context / FBO.
        //
        // NOTE: invalidate_outside_dirty() was previously called here to
        // hint the tiled GPU that clean strips need not be loaded from
        // memory.  Removed because the DrawingBuffer blit (blit_to_surface)
        // reads those regions — the whole surface, or per damage rect on the
        // partial-blit path — and would copy the invalidated (now-undefined)
        // pixels to the window surface on drivers that aggressively discard
        // tiles (Mali, PowerVR).  Buffer-age partial repair further needs the
        // DrawingBuffer and destination pixels to persist, so invalidation
        // must stay off this path.
        let applied = if cm.make_current_needed(canvas_id).is_ok() {
            dirty_region::apply_scissor(gl, &setup.region, setup.canvas_w, setup.canvas_h);
            true
        } else {
            false
        };
        applied
    } else {
        false
    };

    // G-1: pin every image id this batch references so a
    // concurrent `DestroyImage` cannot free the underlying GL
    // texture mid-iteration.  The batch-scope retain is a
    // strict subset of the frame-packet retain (F-1); both
    // paths are safe to use independently because the refcount
    // is additive.  Released unconditionally at the bottom so
    // even an error mid-batch doesn't leak the retain.
    let mut retained_image_ids: smallvec::SmallVec<[u32; 16]> = smallvec::SmallVec::new();
    for cmd in &commands {
        cmd.for_each_referenced_image(|id| {
            cm.retain_in_flight_image(id);
            retained_image_ids.push(id);
        });
    }

    for cmd in commands.drain(..) {
        // Classify damage BEFORE handle_command moves the cmd.
        // Reads Canvas2D state (transform, shadow) from the render thread —
        // this is the authoritative damage source, not the JS-side hint.
        let damage = if is_onscreen {
            cm.get_2d_context_mut(canvas_id)
                .map(|ctx| classify_draw_damage(&cmd, &ctx.renderer.state))
                .unwrap_or(DamageEffect::NoDamage)
        } else {
            DamageEffect::NoDamage
        };

        match renderer_2d.handle_command(cm, canvas_id, cmd) {
            Ok(was_render) => {
                if was_render {
                    batch_dirty = true;
                    cm.add_damage(damage);
                }
            }
            Err(e) => error!("Canvas2DBatch cmd failed: {}", e),
        }
    }

    // G-1: release the batch-scope retain.  If any release drops
    // the refcount to zero *and* a destroy was requested while
    // the image was in flight, `release_in_flight_image`
    // returns the entry so we can `glDeleteTextures` it here.
    // Calls with no pending destroy return `None`, the hot path.
    for id in retained_image_ids.drain(..) {
        if let Some(entry) = cm.release_in_flight_image(id) {
            if let Some(tex) =
                <glow::NativeTexture as NativeTextureFromRawInline>::try_from_raw(entry.gl_texture)
            {
                unsafe { gl.delete_texture(tex) };
            }
        }
    }

    if scissor_applied {
        debug_assert!(
            cm.current_canvas_id() == Some(canvas_id),
            "clear_scissor target mismatch: expected canvas {canvas_id:?} to be current"
        );
        dirty_region::clear_scissor(gl);
    }
    if batch_dirty {
        cm.mark_2d_dirty(canvas_id);
    }

    let should_mark_present = canvas2d_batch_should_mark_present_dirty(
        canvas_id,
        batch_dirty,
        present,
        dirty_rect.is_some(),
    );
    recycle_canvas_command_vec(commands);
    should_mark_present
}

fn execute_gl_batch(
    cm: &mut CanvasManager,
    gl: &glow::Context,
    renderer_gl: &mut RendererGL,
    payload: GlBatchPayload,
) -> bool {
    let mut commands = payload.commands;
    let cmd_count = commands.len();
    let mut batch_hit_onscreen = false;
    let mut error_count: u32 = 0;
    // Collect which canvases this batch touched.  We tally up
    // *before* dispatch (cheap borrow via `&cmd`) so we can scope
    // stale-marking to just the affected contexts and avoid the
    // old "broadcast to every live Canvas2DContext" overkill.
    let mut touched_canvases: std::collections::HashSet<CanvasId> =
        std::collections::HashSet::new();

    for gl_cmd in commands.drain(..) {
        if let Some(cid) = gl_cmd.touches_canvas() {
            touched_canvases.insert(cid);
        }
        match renderer_gl.handle_command(cm, gl, gl_cmd) {
            Ok(effect) => {
                if !matches!(effect, DamageEffect::NoDamage) {
                    batch_hit_onscreen = true;
                }
                cm.add_damage(effect);
            }
            Err(e) => {
                if error_count == 0 {
                    error!("GLBatch cmd failed: {}", e);
                }
                error_count += 1;
            }
        }
    }

    if error_count > 1 {
        warn!("GLBatch: {error_count}/{cmd_count} commands failed");
    }

    // Scoped stale marking: WebGL commands touched GL state; each
    // affected Canvas2DContext's Skia-side cache is now potentially
    // stale, and the next 2D draw on THAT canvas issues exactly one
    // `GrDirectContext::reset()`.  Two-canvas scenes no longer get
    // cross-invalidated just because one canvas ran a WebGL batch.
    //
    // If `touched_canvases` is empty but the batch had commands, it
    // means every command was a resource-context op (CreateShader,
    // LinkProgram, etc.) that doesn't bind any per-canvas state —
    // no Canvas2DContext needs invalidation.
    for cid in touched_canvases {
        cm.mark_2d_context_stale(cid);
    }

    recycle_gl_command_vec(commands);
    batch_hit_onscreen
}

/// Helper: treat an empty GL batch as non-state-mutating so we don't
/// invalidate Skia's cache for a no-op.
#[inline]
fn commands_was_empty(n: usize) -> bool {
    n == 0
}

/// Execute a FramePacket using caller-provided callbacks for each batch type.
/// Used by tests to verify packet structure and ordering without a real GL context.
/// Production code uses `execute_frame_packet` which handles `Materialize` directly.
#[cfg(test)]
pub(crate) fn execute_frame_packet_with_present_tracking<S, FC, FG>(
    packet: FramePacket,
    state: &mut S,
    mut on_canvas: FC,
    mut on_gl: FG,
) -> bool
where
    FC: FnMut(&mut S, CanvasBatchPayload) -> bool,
    FG: FnMut(&mut S, GlBatchPayload) -> bool,
{
    let mut should_present = false;

    for op in packet.into_ops() {
        match op {
            FrameOp::BeginFrame | FrameOp::Present | FrameOp::Materialize { .. } => {}
            FrameOp::CanvasBatch(payload) => {
                should_present |= on_canvas(state, payload);
            }
            FrameOp::GlBatch(payload) => {
                should_present |= on_gl(state, payload);
            }
        }
    }

    should_present
}

#[cfg(test)]
fn execute_frame_packet_with_present_tracking_for_test<S, FC, FG>(
    packet: FramePacket,
    state: &mut S,
    on_canvas: FC,
    on_gl: FG,
) -> bool
where
    FC: FnMut(&mut S, CanvasBatchPayload) -> bool,
    FG: FnMut(&mut S, GlBatchPayload) -> bool,
{
    execute_frame_packet_with_present_tracking(packet, state, on_canvas, on_gl)
}

/// Phase reorder: if the packet's CanvasBatches and GlBatches target
/// disjoint sets of canvases (the cocos shop pattern — offscreen
/// Canvas2D for text labels, onscreen WebGL for the UI), we can run
/// all CanvasBatch + Materialize work as one phase and all GlBatch
/// work as a second phase.  This collapses 100+ EGL context switches
/// (alternating between offscreen canvases and the onscreen canvas)
/// down to roughly `N_distinct_offscreen + 1`.
///
/// Returns `false` whenever the packet might contain a Canvas2D↔WebGL
/// cross-dependency (e.g. `ctx.drawImage(webglCanvasElement, ...)` —
/// the WebGL canvas's pixels must be flushed before the Canvas2D
/// draw reads them), in which case we preserve issue order.
fn packet_safe_to_reorder(ops: &[FrameOp]) -> bool {
    use std::collections::HashSet;
    let mut canvas_targets: HashSet<u32> = HashSet::new();
    let mut gl_targets: HashSet<u32> = HashSet::new();
    for op in ops {
        match op {
            FrameOp::CanvasBatch(payload) => {
                canvas_targets.insert(u32::from(payload.canvas_id));
            }
            FrameOp::GlBatch(payload) => {
                for cmd in &payload.commands {
                    if let Some(cid) = cmd.touches_canvas() {
                        gl_targets.insert(u32::from(cid));
                    }
                }
            }
            _ => {}
        }
    }
    canvas_targets.is_disjoint(&gl_targets)
}

fn execute_frame_packet(
    cm: &mut CanvasManager,
    gl: &glow::Context,
    renderer_2d: &mut Renderer2d,
    renderer_gl: &mut RendererGL,
    packet: FramePacket,
) -> bool {
    let mut should_present = false;

    // F-1: pin every image id this packet references so a
    // concurrent `DestroyImage` defers its `glDeleteTextures`
    // until the Present barrier runs below.  The retained ids
    // are released in strict LIFO order at the end of the
    // function regardless of whether a Present op was present
    // in the packet — otherwise a packet-without-Present (GL-only
    // work, intra-frame Materialize) would leak the retention
    // across the next frame and prevent destroy from ever
    // succeeding.
    let mut retained_image_ids: smallvec::SmallVec<[u32; 16]> = smallvec::SmallVec::new();
    packet.for_each_referenced_image(|id| {
        cm.retain_in_flight_image(id);
        retained_image_ids.push(id);
    });

    // Reorder ops to minimise EGL context switching.  Profiling on
    // hxddd's shop-open scene showed ~430 `eglMakeCurrent` calls per
    // FramePacket — almost all from alternating between ~30 offscreen
    // Canvas2D labels and the onscreen WebGL canvas, ping-pong style.
    // When the packet's Canvas2D and WebGL halves don't share any
    // canvas id, doing all Canvas2D work first then all WebGL work
    // collapses the ping-pong into one switch per distinct canvas
    // (measured: make_current 295 → 152 on the shop scene).  Falls
    // back to issue order when a cross-dependency is detected.
    let ops_vec = packet.into_ops();
    let ops: Vec<FrameOp> = if packet_safe_to_reorder(&ops_vec) {
        let mut phase1: Vec<FrameOp> = Vec::with_capacity(ops_vec.len());
        let mut phase2: Vec<FrameOp> = Vec::with_capacity(ops_vec.len());
        for op in ops_vec {
            match &op {
                FrameOp::BeginFrame | FrameOp::CanvasBatch(_) | FrameOp::Materialize { .. } => {
                    phase1.push(op)
                }
                FrameOp::GlBatch(_) | FrameOp::Present => phase2.push(op),
            }
        }
        phase1.extend(phase2);
        phase1
    } else {
        ops_vec
    };

    for op in ops {
        match op {
            FrameOp::BeginFrame => {}
            // Presentation is coalesced by the physical-frame loop below. Skia
            // maintenance also lives there so multiple packets cannot trigger
            // multiple all-context sweeps in one display frame.
            FrameOp::Present => {}
            FrameOp::Materialize { canvas_id } => {
                // Canvas2D → WebGL boundary.  We MUST:
                //   1. Flush Skia so subsequent GL ops see the pixels.
                //   2. Snapshot/restore the 5 raw-GL bindings Skia
                //      relies on (active-tex, PBO, alignment) via a
                //      `Canvas2DGlScopeGuard`; on drop it also
                //      invalidates the per-canvas dedup shadow so
                //      any value Skia changed under our feet won't
                //      be served from cache next WebGL draw.
                //   3. *Not* eagerly call `reset_gl_state()` — the
                //      per-context `skia_state_stale` flag picks
                //      that up lazily the next time Skia draws.
                //
                // P1-7 invariant: calling `cm.clear_2d_dirty(canvas_id)`
                // here guarantees that the end-of-frame
                // `flush_dirty_2d_contexts` sweep in
                // `present_frame_and_signal_raf` will NOT re-flush the
                // same canvas (it drains `dirty_2d`).  A post-
                // Materialize `Canvas2DBatch` that targets the same
                // canvas will re-mark it dirty via
                // `mark_2d_dirty`, which is the only code path that
                // should trigger a second flush in one frame.
                if cm.make_current_needed(canvas_id).is_ok() {
                    let _gl_scope = cm.begin_canvas2d_gl_scope_for(canvas_id);
                    if let Ok(ctx) = cm.get_2d_context_mut(canvas_id) {
                        ctx.flush_and_submit();
                    }
                    renderer_2d.clear_dirty_layer(canvas_id);
                    cm.clear_2d_dirty(canvas_id);
                    // `_gl_scope` drops here → raw-GL state restored,
                    // dedup shadow invalidated.
                }
            }
            FrameOp::CanvasBatch(payload) => {
                should_present |= execute_canvas_batch(cm, gl, renderer_2d, payload);
            }
            FrameOp::GlBatch(payload) => {
                should_present |= execute_gl_batch(cm, gl, renderer_gl, payload);
            }
        }
    }

    // F-1 release-then-drain sequence.  Release every id the
    // packet retained so any `DestroyImage` that arrived during
    // the frame can finally free its GL texture; then call
    // `drain_pending_image_deletions` which actually issues the
    // `glDeleteTextures` for ids whose refcount just reached
    // zero.  If the release returns `Some(entry)` directly,
    // another caller's drain missed the release window; we
    // free it here anyway so no entry leaks.
    for id in retained_image_ids.drain(..) {
        if let Some(entry) = cm.release_in_flight_image(id) {
            if let Some(tex) =
                <glow::NativeTexture as NativeTextureFromRawInline>::try_from_raw(entry.gl_texture)
            {
                unsafe { gl.delete_texture(tex) };
            }
        }
    }
    cm.drain_pending_image_deletions();

    should_present
}

/// Private shim mirroring the one in `canvas::manager::mod.rs`;
/// kept here so `execute_frame_packet` can reconstruct a
/// `NativeTexture` without cross-module visibility gymnastics.
trait NativeTextureFromRawInline {
    fn try_from_raw(raw: u32) -> Option<glow::NativeTexture>;
}
impl NativeTextureFromRawInline for glow::NativeTexture {
    #[inline]
    fn try_from_raw(raw: u32) -> Option<glow::NativeTexture> {
        std::num::NonZeroU32::new(raw).map(glow::NativeTexture)
    }
}

fn next_vsync_frame_decision(
    scheduler: &mut FrameScheduler,
    surface: &SurfaceSystem,
    frame_time_ms: f64,
) -> VsyncFrameDecision {
    let decision = scheduler.on_vsync(frame_time_ms);
    VsyncFrameDecision {
        should_signal_raf: decision.should_render,
        should_present: decision.should_render && surface.can_present(),
        raf_time_ms: decision.raf_time_ms,
    }
}

#[cfg(test)]
fn finalize_vsync_frame_decision<F>(
    scheduler: &mut FrameScheduler,
    surface: &mut SurfaceSystem,
    frame_time_ms: f64,
    apply_pending: F,
) -> VsyncFrameDecision
where
    F: FnOnce(&mut SurfaceSystem),
{
    let decision = scheduler.on_vsync(frame_time_ms);
    apply_pending(surface);
    VsyncFrameDecision {
        should_signal_raf: decision.should_render,
        should_present: decision.should_render && surface.can_present(),
        raf_time_ms: decision.raf_time_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        canvas2d_batch_should_mark_present_dirty,
        execute_frame_packet_with_present_tracking_for_test, finalize_vsync_frame_decision,
        mark_surface_destroyed, next_vsync_frame_decision, report_recovery_failure,
    };
    use crate::{SurfaceSystem, frame_scheduler::FrameScheduler};
    use shared::protocol::render_cmd::{Canvas2DCmd, CanvasBatchPayload, DirtyRect};
    use shared::{FrameOp, FramePacketBuilder};

    #[test]
    fn recovery_failure_report_and_log_gate_once_per_loss_epoch() {
        let context = std::sync::Arc::new(shared::op_state::ContextLostState::default());
        assert!(context.set_lost());
        let (events, _event_rx) = shared::render_event::channel();
        let mut last_epoch = u64::MAX;

        assert!(report_recovery_failure(&context, &events, &mut last_epoch,));
        assert!(!report_recovery_failure(&context, &events, &mut last_epoch,));

        assert!(context.set_recovered());
        assert!(context.set_lost());
        assert!(report_recovery_failure(&context, &events, &mut last_epoch,));
    }

    // R1 demand-model contracts. These combine the real `RafDemand` latch with
    // the pure arm-decision helpers to pin the render-thread behaviour the
    // present/arm wiring depends on. (They run under `cargo ndk test -p graphics`
    // on device; on host the graphics test binary cannot link freetype/EGL.)
    use crate::frame_scheduler::{raf_demand_remains, should_arm_one_shot};
    use shared::raf_signal::RafDemand;

    #[test]
    fn raf_signalled_only_when_waiter_pending_and_consumed() {
        let demand = RafDemand::new();
        // Dirty-only / upload-only frame: no waiter -> must NOT signal RAF.
        assert!(
            demand.take_waiter().is_none(),
            "dirty-only / upload-only frame does not signal RAF"
        );
        // A pending waiter is signalled exactly once and consumed.
        let ticket = demand.mark_waiting();
        assert_eq!(
            demand.take_waiter(),
            Some(ticket),
            "waiter pending -> signal"
        );
        assert!(
            demand.take_waiter().is_none(),
            "already consumed -> no second signal"
        );
    }

    #[test]
    fn failed_signal_restores_waiter() {
        let demand = RafDemand::new();
        let ticket = demand.mark_waiting();
        assert_eq!(demand.take_waiter(), Some(ticket));
        // Simulate raf_tx.signal(ts) == false (eventfd/channel full):
        demand.restore_waiter(ticket);
        assert!(
            demand.is_waiting(),
            "failed signal re-arms so RAF is not frozen"
        );
    }

    #[test]
    fn skipped_vsync_with_pending_waiter_rearms_until_deadline() {
        let demand = RafDemand::new();
        demand.mark_waiting();
        // FrameScheduler skipped this physical vsync (should_signal_raf=false):
        // the waiter is NOT consumed, so demand remains and we must re-arm to
        // keep receiving physical vsyncs until the target-FPS deadline is hit.
        let remains = raf_demand_remains(demand.is_waiting(), false, false, false);
        assert!(remains, "pending waiter keeps demand after a skipped vsync");
        assert!(
            should_arm_one_shot(true, true, false, true, remains, false),
            "skipped vsync with a pending waiter re-arms the one-shot"
        );
        // Paused / no live surface must block the arm even with demand.
        assert!(
            !should_arm_one_shot(true, true, true, true, remains, false),
            "paused blocks arm"
        );
        assert!(
            !should_arm_one_shot(true, true, false, false, remains, false),
            "no surface blocks arm"
        );
    }

    #[test]
    fn vsync_path_uses_scheduler_and_surface_state_for_presentation() {
        let mut scheduler = FrameScheduler::new(60);
        let mut surface = SurfaceSystem::new();

        let first = next_vsync_frame_decision(&mut scheduler, &surface, 0.0);
        assert!(first.should_signal_raf);
        assert!(!first.should_present);
        assert_eq!(first.raf_time_ms, 0.0);

        surface.on_surface_available((1080, 1920));

        let second = next_vsync_frame_decision(&mut scheduler, &surface, 16.667);
        assert!(second.should_signal_raf);
        assert!(second.should_present);
        assert!(second.raf_time_ms > 0.0);
    }

    #[test]
    fn surface_destroyed_blocks_presentation_until_surface_recreated() {
        let mut scheduler = FrameScheduler::new(60);
        let mut surface = SurfaceSystem::new();
        surface.on_surface_available((1080, 1920));

        let first = next_vsync_frame_decision(&mut scheduler, &surface, 0.0);
        assert!(first.should_present);

        mark_surface_destroyed(&mut surface);

        let second = next_vsync_frame_decision(&mut scheduler, &surface, 16.667);
        assert!(second.should_signal_raf);
        assert!(!second.should_present);
    }

    #[test]
    fn pending_surface_destroyed_prevents_presentation_in_same_vsync_iteration() {
        let mut scheduler = FrameScheduler::new(60);
        let mut surface = SurfaceSystem::new();
        surface.on_surface_available((1080, 1920));

        let decision =
            finalize_vsync_frame_decision(&mut scheduler, &mut surface, 0.0, |surface| {
                mark_surface_destroyed(surface);
            });

        assert!(decision.should_signal_raf);
        assert!(!decision.should_present);
    }

    #[test]
    fn non_presenting_canvas2d_batch_does_not_mark_present_dirty() {
        assert!(!canvas2d_batch_should_mark_present_dirty(
            1, true, false, true
        ));
        assert!(canvas2d_batch_should_mark_present_dirty(
            1, true, true, true
        ));
        assert!(canvas2d_batch_should_mark_present_dirty(
            1, false, true, true
        ));
        assert!(!canvas2d_batch_should_mark_present_dirty(
            1, false, true, false
        ));
    }

    #[test]
    fn frame_packet_presenting_canvas_batch_without_present_op_still_requests_present() {
        let packet = FramePacketBuilder::new(1, 16.6)
            .push(FrameOp::BeginFrame)
            .push(FrameOp::CanvasBatch(CanvasBatchPayload {
                canvas_id: 1,
                commands: vec![Canvas2DCmd::Save],
                present: true,
                dirty_rect: None,
            }))
            .finish();

        let mut canvas_exec_count = 0;
        let mut gl_exec_count = 0;

        let should_present = execute_frame_packet_with_present_tracking_for_test(
            packet,
            &mut (),
            |(), payload| {
                canvas_exec_count += 1;
                assert!(payload.present);
                true
            },
            |(), _payload| {
                gl_exec_count += 1;
                false
            },
        );

        assert!(should_present);
        assert_eq!(canvas_exec_count, 1);
        assert_eq!(gl_exec_count, 0);
    }

    #[test]
    fn frame_packet_presenting_canvas_batch_executes_canvas_work_and_requests_present() {
        let packet = FramePacketBuilder::new(1, 16.6)
            .push(FrameOp::BeginFrame)
            .push(FrameOp::CanvasBatch(CanvasBatchPayload {
                canvas_id: 1,
                commands: vec![Canvas2DCmd::Save],
                present: true,
                dirty_rect: Some(DirtyRect {
                    x: 0.0,
                    y: 0.0,
                    width: 8.0,
                    height: 8.0,
                }),
            }))
            .push(FrameOp::Present)
            .finish();

        #[derive(Default)]
        struct PacketExecStats {
            canvas_exec_count: usize,
            gl_exec_count: usize,
        }

        let mut stats = PacketExecStats::default();

        let should_present = execute_frame_packet_with_present_tracking_for_test(
            packet,
            &mut stats,
            |stats, payload| {
                stats.canvas_exec_count += 1;
                assert_eq!(payload.canvas_id, 1);
                assert!(payload.present);
                assert_eq!(payload.commands.len(), 1);
                true
            },
            |stats, _payload| {
                stats.gl_exec_count += 1;
                false
            },
        );

        assert!(should_present);
        assert_eq!(stats.canvas_exec_count, 1);
        assert_eq!(stats.gl_exec_count, 0);
    }

    #[test]
    fn frame_packet_state_only_canvas_batch_does_not_request_present_from_explicit_boundary() {
        let packet = FramePacketBuilder::new(2, 16.6)
            .push(FrameOp::BeginFrame)
            .push(FrameOp::CanvasBatch(CanvasBatchPayload {
                canvas_id: 1,
                commands: vec![Canvas2DCmd::Save],
                present: true,
                dirty_rect: None,
            }))
            .push(FrameOp::Present)
            .finish();

        let should_present = execute_frame_packet_with_present_tracking_for_test(
            packet,
            &mut (),
            |(), payload| {
                assert!(payload.present);
                false
            },
            |(), _payload| {
                panic!("unexpected gl batch");
            },
        );

        assert!(!should_present);
    }

    #[test]
    fn frame_packet_gl_work_requests_present_without_explicit_present_op() {
        let packet = FramePacketBuilder::new(3, 16.6)
            .push(FrameOp::BeginFrame)
            .push(FrameOp::GlBatch(
                shared::protocol::render_cmd::GlBatchPayload {
                    commands: Vec::new(),
                },
            ))
            .finish();

        let should_present = execute_frame_packet_with_present_tracking_for_test(
            packet,
            &mut (),
            |(), _payload| {
                panic!("unexpected canvas batch");
            },
            |(), _payload| true,
        );

        assert!(should_present);
    }

    // ── prepare_batch_scissor tests ──

    #[test]
    fn prepare_batch_scissor_returns_none_when_not_presenting() {
        use super::prepare_batch_scissor;
        let rect = Some(DirtyRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        });
        assert!(prepare_batch_scissor(false, &rect, Some((800, 600))).is_none());
    }

    #[test]
    fn prepare_batch_scissor_returns_none_when_dirty_rect_is_none() {
        use super::prepare_batch_scissor;
        assert!(prepare_batch_scissor(true, &None, Some((800, 600))).is_none());
    }

    #[test]
    fn prepare_batch_scissor_returns_none_when_canvas_size_unavailable() {
        use super::prepare_batch_scissor;
        let rect = Some(DirtyRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        });
        assert!(prepare_batch_scissor(true, &rect, None).is_none());
    }

    #[test]
    fn prepare_batch_scissor_returns_region_when_all_conditions_met() {
        use super::prepare_batch_scissor;
        let rect = Some(DirtyRect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        });
        let setup = prepare_batch_scissor(true, &rect, Some((800, 600)));
        assert!(setup.is_some());
        let s = setup.unwrap();
        assert_eq!(s.canvas_w, 800);
        assert_eq!(s.canvas_h, 600);
        assert_eq!(s.region.x, 10);
        assert_eq!(s.region.width, 100);
        assert_eq!(s.region.height, 50);
    }

    #[test]
    fn prepare_batch_scissor_flips_y_from_top_left_to_gl_bottom_left_origin() {
        use super::prepare_batch_scissor;
        // DirtyRect: top-left origin, y=20, height=50 → bottom edge at y=70
        // GL scissor: bottom-left origin → y = canvas_h - bottom_edge = 600 - 70 = 530
        let rect = Some(DirtyRect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        });
        let s = prepare_batch_scissor(true, &rect, Some((800, 600))).unwrap();
        assert_eq!(s.region.y, 530);
    }

    #[test]
    fn prepare_batch_scissor_clamps_negative_y_to_zero() {
        use super::prepare_batch_scissor;
        // DirtyRect extends past canvas bottom: y=580, h=100 → bottom_edge=680
        // GL y = (600 - 680).max(0) = 0
        let rect = Some(DirtyRect {
            x: 0.0,
            y: 580.0,
            width: 100.0,
            height: 100.0,
        });
        let s = prepare_batch_scissor(true, &rect, Some((800, 600))).unwrap();
        assert_eq!(s.region.y, 0);
    }

    #[test]
    fn prepare_batch_scissor_ceils_fractional_dimensions() {
        use super::prepare_batch_scissor;
        let rect = Some(DirtyRect {
            x: 10.3,
            y: 20.7,
            width: 99.1,
            height: 49.9,
        });
        let s = prepare_batch_scissor(true, &rect, Some((800, 600))).unwrap();
        // x floors: 10.3 → 10
        assert_eq!(s.region.x, 10);
        // width ceils: 99.1 → 100
        assert_eq!(s.region.width, 100);
        // height ceils: 49.9 → 50
        assert_eq!(s.region.height, 50);
    }

    // ── Materialize / mixed-frame ordering tests ──

    #[test]
    fn frame_packet_materialize_op_does_not_affect_present_decision() {
        let packet = FramePacketBuilder::new(1, 16.6)
            .push(FrameOp::BeginFrame)
            .push(FrameOp::CanvasBatch(CanvasBatchPayload {
                canvas_id: 1,
                commands: vec![Canvas2DCmd::Save],
                present: true,
                dirty_rect: None,
            }))
            .push(FrameOp::Materialize { canvas_id: 1 })
            .push(FrameOp::GlBatch(
                shared::protocol::render_cmd::GlBatchPayload {
                    commands: Vec::new(),
                },
            ))
            .push(FrameOp::Present)
            .finish();

        let should_present = execute_frame_packet_with_present_tracking_for_test(
            packet,
            &mut (),
            |(), _payload| true,
            |(), _payload| false,
        );

        assert!(should_present);
    }

    #[test]
    fn frame_packet_executes_interleaved_ops_in_submission_order() {
        let packet = FramePacketBuilder::new(1, 16.6)
            .push(FrameOp::BeginFrame)
            .push(FrameOp::CanvasBatch(CanvasBatchPayload {
                canvas_id: 1,
                commands: vec![Canvas2DCmd::Save],
                present: false,
                dirty_rect: None,
            }))
            .push(FrameOp::Materialize { canvas_id: 1 })
            .push(FrameOp::GlBatch(
                shared::protocol::render_cmd::GlBatchPayload {
                    commands: Vec::new(),
                },
            ))
            .push(FrameOp::CanvasBatch(CanvasBatchPayload {
                canvas_id: 1,
                commands: vec![Canvas2DCmd::Restore],
                present: true,
                dirty_rect: None,
            }))
            .push(FrameOp::Present)
            .finish();

        let mut order: Vec<&'static str> = Vec::new();
        let should_present = execute_frame_packet_with_present_tracking_for_test(
            packet,
            &mut order,
            |order, _payload| {
                order.push("canvas");
                true
            },
            |order, _payload| {
                order.push("gl");
                false
            },
        );

        assert!(should_present);
        assert_eq!(order, vec!["canvas", "gl", "canvas"]);
    }
}

impl RenderThread {
    /// Spawn render thread.
    ///
    /// * `raf_tx` — frame timestamp sender (render → JS async op).
    ///   On Android this is eventfd-backed; on other platforms, tokio mpsc.
    /// * `vsync_rx` — optional crossbeam receiver for Choreographer VSync signals
    /// * `host_id` — host identifier for debug stats registry
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        raf_tx: RafSender,
        vsync_rx: Option<Receiver<f64>>,
        host_id: i32,
        initial_surface: Option<SurfaceRef>,
        dpi: f32,
        app_cache_dir: Option<std::path::PathBuf>,
        gpu_caps: std::sync::Arc<shared::device::gpu_caps::GpuCaps>,
        // Authoritative GL-context-loss state, shared with the host runtime and
        // JS `op_gl_is_context_lost`. The render thread is the *only* writer and
        // updates it edge-triggered: `lost` false->true on a real reset / EGL
        // loss, true->false on a probe-verified recovery, bumping `epoch` on
        // each transition. Because the render-event channel is lossy, this state
        // -- not the event -- is the source of truth for `gl.isContextLost()`,
        // and `epoch` lets the host recover event edges that the channel dropped.
        context_lost: Arc<shared::op_state::ContextLostState>,
        // Wake callback fired after EVERY successfully enqueued render event, so
        // Canvas/GL/swap/RAF-backpressure/context feedback reaches the host
        // without a polling timer. The host supplies a `tokio::sync::Notify`-
        // backed closure (coalescing bursts to one permit); `None` falls back to
        // the no-wake channel. Decoupled from `HostCommand` so the graphics crate
        // stays independent of the host command enum.
        wake: Option<Arc<dyn Fn() + Send + Sync>>,
        // R1: shared RAF waiter demand latch. `op_await_next_frame` publishes a
        // waiter before awaiting; the render thread consumes it (and only then
        // signals RAF) so dirty-only / upload-only frames never write an
        // unconsumed timestamp, and a failed signal restores it so RAF cannot
        // freeze. Shared `Arc` survives JS soft restart with the RafReceiver.
        raf_demand: shared::raf_signal::RafDemandRef,
        // R1: one-shot vsync arm. `Some` on platforms with a demand-driven
        // display clock (Android Choreographer via a native->Java route);
        // `None` on the software-ticker path and in tests. The render thread
        // calls it to request exactly one more frame while demand remains;
        // graphics stays decoupled from `platform` (it only invokes a closure).
        request_vsync: Option<Arc<dyn Fn() + Send + Sync>>,
        // Per-host surface destroy-epoch (monotonic), advanced by the JNI/UI
        // thread on every surfaceDestroyed. Each SurfaceRef carries the epoch it
        // was created at (Surface::surface_epoch); the render loop compares its
        // current surface's epoch against this live counter every frame and stops
        // presenting on any mismatch — synchronously and independent of the async
        // SurfaceDestroyed command. A monotonic counter (vs a boolean) is required
        // to survive fast destroy->create->destroy without ABA masking.
        destroy_epoch: Arc<std::sync::atomic::AtomicU64>,
    ) -> EngineResult<Self> {
        let (cmd_tx, cmd_rx) = CommandSender::new();
        // Wire the host wake into the event channel so every successful emit
        // signals the host (coalesced by the host's Notify); no manual per-event
        // nudge remains below.
        let (event_tx, event_rx) = match wake {
            Some(w) => shared::render_event::channel_with_wake(w),
            None => shared::render_event::channel(),
        };

        // F-2: build the shared TextContext here (no GL deps —
        // SkFontMgr + SkFontCollection are pure CPU objects) so
        // both halves — the render thread's `Renderer2d` and the
        // type-erased handle we expose on `RenderThread` — pick
        // up the same `Arc`.
        let (shared_text_ctx, text_measurer) = crate::text_measurer_impl::into_shared_measurer(
            crate::backend::gl::text::TextContext::new(),
        );

        let event_tx_thread = event_tx.clone();
        let shared_text_ctx_for_thread = shared_text_ctx.clone();
        let shutdown_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shutdown_requested_for_owner = shutdown_requested.clone();
        let handle = std::thread::Builder::new()
            .name("Migo-RenderThread".into())
            .spawn(move || {
                let events: RenderEventSender = event_tx_thread;
                let shared_text_ctx = shared_text_ctx_for_thread;
                shared::thread_priority::set_current_thread_priority(
                    shared::thread_priority::Priority::Display,
                );
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // Both Android and non-Android use the same EGL library name.
                const EGL_LIB: &str = "libEGL.so";

                let mut cm = match CanvasManager::new_with_resource(EGL_LIB, dpi, app_cache_dir.as_deref(), gpu_caps.clone()) {
                    Ok(c) => c,
                    Err(e) => {
                        gpu_caps.set_failed(format!("CanvasManager init failed: {}", e));
                        error!("CanvasManager init failed: {}", e);
                        return;
                    }
                };

                let has_initial_surface = initial_surface.is_some();
                let initial_surface_size = initial_surface.as_ref().map(|surface| surface.size());
                let mut initial_onscreen_ok = false;
                let mut startup_failed = false;

                // If an initial surface is provided, create the onscreen context immediately.
                if let Some(surface) = initial_surface {
                    match onscreen_window_from_surface(surface.as_ref()) {
                        Ok(win) => {
                            if let Err(e) = cm.create_onscreen(win, None) {
                                error!("create_onscreen failed: {}", e);
                                gpu_caps.set_failed(format!("create_onscreen failed: {}", e));
                                startup_failed = true;
                            } else {
                                initial_onscreen_ok = true;
                            }
                        }
                        Err(e) => {
                            gpu_caps.set_failed(format!("initial surface is not supported: {}", e));
                            error!("initial surface is not supported: {}", e);
                            startup_failed = true;
                        }
                    }
                }

                // Publish Ready only after every startup step represented by the
                // host's GPU join has succeeded. A surface failure has already
                // published Failed and must never be overwritten by Ready.
                if !startup_failed {
                    cm.publish_gpu_caps();
                }

                // Create GL function loader.
                let gl = unsafe {
                    glow::Context::from_loader_function(|s| {
                        cm.egl
                            .get_proc_address(s)
                            .map(|f| f as *const std::ffi::c_void)
                            .unwrap_or(std::ptr::null())
                    })
                };

                // ---- Timing sources ----
                // When Choreographer VSync is available, use it as primary timing.
                // Otherwise fall back to a software ticker at the configured FPS.
                let has_vsync = vsync_rx.is_some();
                let vsync: Receiver<f64> = vsync_rx.unwrap_or_else(crossbeam_channel::never);

                let mut fps: u32 = 60;
                let mut ticker: Receiver<Instant> = if has_vsync {
                    crossbeam_channel::never()
                } else {
                    tick(Duration::from_secs_f32(1.0 / fps as f32))
                };
                let mut frame_scheduler = FrameScheduler::new(fps);

                let start_time = Instant::now();
                let cleanup_cadence = DeferredCleanupCadence::new(start_time.elapsed());
                let mut dirty = true;
                let mut paused = false;
                // R1: whether a one-shot vsync callback has been requested but not
                // yet delivered. Suppresses redundant `request_vsync` JNI calls
                // while a callback is already in flight; reset when a vsync is
                // delivered or on any lifecycle edge (Pause/Resume/SurfaceDestroyed/
                // RecreateOnscreen) where Java unschedules/reschedules. `Cell` so
                // the command-handling closure can reset it via shared capture.
                let vsync_armed = std::cell::Cell::new(false);
                let mut surface_system = SurfaceSystem::new();
                // Epoch of the surface the render thread currently holds. Compared
                // against the live `destroy_epoch` each frame; a mismatch means the
                // surface was destroyed after hand-off. Init 0 to match the initial
                // surface's epoch and the destroy counter's initial value. `Cell`
                // so the command-handling closures can update it via shared capture
                // (they already borrow the environment) without a unique borrow.
                let valid_epoch = std::cell::Cell::new(0u64);
                if initial_onscreen_ok {
                    if let Some(size) = initial_surface_size {
                        surface_system.on_surface_available(size);
                    } else if has_initial_surface {
                        surface_system.on_resume();
                    }
                }

                let mut canvas_handler = CanvasHandler::new();
                let mut renderer_2d = Renderer2d::from_shared_text(shared_text_ctx);
                let mut renderer_gl = RendererGL::new();
                let mut render_server = RenderServer::new();

                // ---- FPS stats ----
                let debug_stats = shared::stats::register_stats(host_id);
                crate::render_diagnostics::install(debug_stats.clone());
                let mut frame_count: u32 = 0;
                let mut fps_timer = Instant::now();
                let mut last_frame_time = Instant::now();
                let mut first_frame_recorded = false;

                // Deferred EGL context recovery flag. Set to true when
                // swap_buffers detects context loss (EGL_CONTEXT_LOST).
                // Recovery is deferred to the top of the next frame iteration
                // instead of running inline during present, because
                // try_recover_context() performs a full EGL surface+context
                // teardown and recreate, which is too expensive to run in the
                // time-critical swap path where it delays the RAF signal and
                // can cause cascading frame drops.
                let mut needs_context_recovery = false;

                // Epoch of the last context-loss episode we already reported a
                // *recovery failure* for. Recovery is retried every frame while
                // lost; without this, each failed retry would emit a
                // ContextRecovered{success:false} + signal the host + trigger a
                // Java notify_error at up to 60 fps. We report the failure once
                // per loss episode (the epoch bumps on each fresh loss), then
                // stay quiet until recovery succeeds or a new loss occurs.
                let mut last_recovery_fail_epoch: u64 = u64::MAX;

                // Diag: track when the render thread entered pause
                // so `Resume` can report wall-clock paused duration.
                // Useful for correlating "came back black" bug
                // reports with Android TRIM_MEMORY / OOM kill
                // timelines.
                //
                // Interior-mutability (`Cell`) because the handler
                // closure captures immutably — wrapping avoids the
                // blast radius of flipping the closure to FnMut.
                let pause_started_at: core::cell::Cell<Option<Instant>> =
                    core::cell::Cell::new(None);

                enum LoopCtl {
                    Continue,
                    Shutdown,
                }

                let handle_one_cmd = |cmd: RenderCommand,
                                          cm: &mut CanvasManager,
                                          gl: &glow::Context,
                                          canvas_handler: &mut CanvasHandler,
                                          renderer_2d: &mut Renderer2d,
                                           renderer_gl: &mut RendererGL,
                                           fps: &mut u32,
                                           frame_scheduler: &mut FrameScheduler,
                                           ticker: &mut Receiver<Instant>,
                                           dirty: &mut bool,
                                           paused: &mut bool,
                                           has_vsync: bool,
                                           surface_system: &mut SurfaceSystem,
                                           render_server: &mut RenderServer|
                 -> LoopCtl {
                    match cmd {
                        RenderCommand::Shutdown => {
                            info!("RenderThread received Shutdown");
                            cm.destroy_all(&gl);
                            return LoopCtl::Shutdown;
                        }

                        RenderCommand::FrameRate(new_fps) => {
                            let new_fps = new_fps.clamp(1, 120);
                            if has_vsync {
                                frame_scheduler.set_preferred_fps(new_fps);
                                info!("RenderThread target fps changed to {} (scheduler)", new_fps);
                            } else if new_fps != *fps {
                                // Software ticker mode: recreate ticker at new interval.
                                *fps = new_fps;
                                if !*paused {
                                    *ticker = tick(Duration::from_secs_f32(1.0 / *fps as f32));
                                }
                                info!("RenderThread fps changed to {}", fps);
                            }
                        }

                        RenderCommand::Canvas(canvas_cmd) => {
                            // Only mark dirty for commands that affect onscreen visual output.
                            let is_recreate = matches!(&canvas_cmd, shared::protocol::render_cmd::CanvasCmd::RecreateOnscreen { .. });
                            let recreate_surface_size = match &canvas_cmd {
                                shared::protocol::render_cmd::CanvasCmd::RecreateOnscreen {
                                    surface,
                                    ..
                                } => Some(surface.size()),
                                _ => None,
                            };
                            // Epoch stamped on the surface at updateSurface time —
                            // becomes this render thread's valid_epoch once the
                            // surface is successfully recreated.
                            let recreate_surface_epoch = match &canvas_cmd {
                                shared::protocol::render_cmd::CanvasCmd::RecreateOnscreen {
                                    surface,
                                    ..
                                } => Some(surface.surface_epoch()),
                                _ => None,
                            };
                            let affects_onscreen = match &canvas_cmd {
                                shared::protocol::render_cmd::CanvasCmd::RecreateOnscreen { .. } => true,
                                shared::protocol::render_cmd::CanvasCmd::ResizeCanvas { id, .. } => *id == 1,
                                _ => false,
                            };
                            // If the incoming surface's destroy-epoch differs from the
                            // one we currently hold, a surfaceDestroyed occurred since:
                            // the old onscreen EGLSurface is bound to an abandoned
                            // window. Force a full recreate so create_onscreen's
                            // same-window fast paths can't keep presenting to the dead
                            // surface. (Same epoch => a benign resize/redundant call,
                            // where the fast path safely avoids an EGL re-connect.)
                            if is_recreate {
                                if let Some(e) = recreate_surface_epoch {
                                    if e != valid_epoch.get() {
                                        cm.force_next_onscreen_recreate();
                                    }
                                }
                            }
                            match canvas_handler.handle_command(cm, canvas_cmd) {
                                Ok(()) => {
                                    if is_recreate {
                                        if let Some(size) = recreate_surface_size {
                                            surface_system.on_surface_available(size);
                                            // R1: surface is back — clear the
                                            // in-flight flag so the post-recreate
                                            // dirty frame re-arms the clock.
                                            vsync_armed.set(false);
                                            // Adopt the recreated surface's epoch. From now on the
                                            // present-gate compares this against the live destroy
                                            // counter, so a destroy that raced this recreate (bumped
                                            // the counter past this epoch) still stops presenting —
                                            // no ABA, because a monotonic epoch can't read "equal"
                                            // against a newer surface generation.
                                            if let Some(e) = recreate_surface_epoch {
                                                valid_epoch.set(e);
                                            }
                                            info!(
                                                width = size.0,
                                                height = size.1,
                                                surface_state = ?surface_system.state(),
                                                canvas_count = cm.canvas_count(),
                                                "RenderThread surface recreated"
                                            );
                                        } else {
                                            info!(
                                                surface_state = ?surface_system.state(),
                                                canvas_count = cm.canvas_count(),
                                                "RenderThread surface recreated (no size reported)"
                                            );
                                        }
                                    }
                                    if affects_onscreen {
                                        *dirty = true;
                                    }
                                }
                                Err(e) => {
                                    error!("CanvasCmd failed: {}", e);
                                    debug_stats
                                        .canvas_cmd_errors
                                        .fetch_add(1, Ordering::Relaxed);
                                    events.emit(RenderEvent::CanvasError {
                                        code: e.code,
                                        message: e.to_string(),
                                    });
                                }
                            }
                        }

                        RenderCommand::GL(gl_cmd) => match renderer_gl.handle_command(cm, gl, gl_cmd) {
                            Ok(effect) => {
                                if !matches!(effect, DamageEffect::NoDamage) {
                                    *dirty = true;
                                }
                                cm.add_damage(effect);
                            }
                            Err(e) => {
                                error!("GLCmd failed: {}", e);
                                debug_stats
                                    .gl_cmd_errors
                                    .fetch_add(1, Ordering::Relaxed);
                                events.emit(RenderEvent::GlError {
                                    code: e.code,
                                    message: e.to_string(),
                                });
                            }
                        },
                        RenderCommand::GLBatch(payload) => {
                            if execute_gl_batch(cm, gl, renderer_gl, payload) {
                                *dirty = true;
                            }
                        }

                        RenderCommand::Canvas2D { canvas_id, cmd } => {
                            // G-1: retain referenced image ids around
                            // the dispatch so a concurrent `DestroyImage`
                            // still waits for the draw to land.
                            // SmallVec-backed; zero allocation for the
                            // common `DrawImage { image_id }` case.
                            let mut retained: smallvec::SmallVec<[u32; 4]> =
                                smallvec::SmallVec::new();
                            cmd.for_each_referenced_image(|id| {
                                cm.retain_in_flight_image(id);
                                retained.push(id);
                            });
                            let result = renderer_2d.handle_command(cm, canvas_id, cmd);
                            for id in retained.drain(..) {
                                if let Some(entry) = cm.release_in_flight_image(id) {
                                    if let Some(tex) =
                                        <glow::NativeTexture as NativeTextureFromRawInline>::try_from_raw(
                                            entry.gl_texture,
                                        )
                                    {
                                        unsafe { gl.delete_texture(tex) };
                                    }
                                }
                            }
                            match result {
                                Ok(was_render) => {
                                    if was_render {
                                        cm.mark_2d_dirty(canvas_id);
                                        // NOTE: dirty flag is NOT set here. Canvas2D commands
                                        // may arrive mid-frame (e.g. _frameEnd() before
                                        // measureText). Setting dirty here would cause the
                                        // render thread to present a partial frame on the next
                                        // VSync, producing visible flicker. Instead, the JS
                                        // frame-end (op_frame_end_all) sends an explicit
                                        // Invalidate to trigger the present.
                                    }
                                }
                                Err(e) => {
                                    error!("Canvas2D failed: {}", e);
                                    debug_stats
                                        .canvas2d_cmd_errors
                                        .fetch_add(1, Ordering::Relaxed);
                                    events.emit(RenderEvent::Canvas2DError {
                                        code: e.code,
                                        message: e.to_string(),
                                    });
                                }
                            }
                        }

                        // V2: Batched commands - process all commands in a single frame
                        RenderCommand::Canvas2DBatch(payload) => {
                            if execute_canvas_batch(cm, gl, renderer_2d, payload) {
                                *dirty = true;
                            }
                        }

                        RenderCommand::FramePacket(mut packet) => {
                            render_server.stamp_packet(&mut packet);
                            if execute_frame_packet(cm, gl, renderer_2d, renderer_gl, packet) {
                                *dirty = true;
                            }
                        }

                        // Invalidate signal for on-demand rendering
                        RenderCommand::Invalidate => {
                            *dirty = true;
                        }

                        RenderCommand::Pause => {
                            if !*paused {
                                *paused = true;
                                // R1: Java stops the scheduler on pause (removing
                                // any posted callback); clear our in-flight flag so
                                // Resume re-arms retained demand.
                                vsync_armed.set(false);
                                let live_canvases = cm.canvas_count();
                                let surface_state_before = surface_system.state();
                                surface_system.on_pause();
                                if !has_vsync {
                                    *ticker = crossbeam_channel::never();
                                }
                                // R-6: the game just went to background.
                                // Release GPU memory aggressively so the
                                // Android lowmemorykiller does not evict
                                // the whole process under memory
                                // pressure.  `on_trim_memory` lowers the
                                // Skia resource-cache budget AND runs a
                                // synchronous deferred-cleanup sweep on
                                // every live `Canvas2DContext`, matching
                                // the Chromium Android idle idiom.
                                // Resume will rehydrate caches on first
                                // draw; the one-time CPU upload cost is
                                // vastly preferred to being killed and
                                // having to restart the engine.
                                cm.on_trim_memory();
                                cleanup_cadence.mark_forced(start_time.elapsed());
                                pause_started_at.set(Some(Instant::now()));
                                info!(
                                    live_canvases,
                                    has_vsync,
                                    surface_state_before = ?surface_state_before,
                                    "RenderThread paused; ran aggressive GPU cache trim"
                                );
                            }
                        }

                        RenderCommand::Resume => {
                            if *paused {
                                *paused = false;
                                // R1: clear in-flight flag so the forced first-frame
                                // dirty below re-arms even if a stale callback was
                                // dropped while paused.
                                vsync_armed.set(false);
                                let paused_ms = pause_started_at
                                    .take()
                                    .map(|t: Instant| t.elapsed().as_millis() as u64)
                                    .unwrap_or(0);
                                let surface_state_before = surface_system.state();
                                surface_system.on_resume();
                                let surface_state_after = surface_system.state();
                                if !has_vsync {
                                    *ticker = tick(Duration::from_secs_f32(1.0 / *fps as f32));
                                }
                                // Request a frame so the first post-
                                // resume paint isn't gated on the game
                                // JS happening to dirty something.
                                // Without this, on some devices the
                                // surface comes back but the canvas
                                // stays on the pre-pause frame until
                                // the next user interaction — looks
                                // like "black / frozen on resume".
                                *dirty = true;
                                info!(
                                    paused_ms,
                                    has_vsync,
                                    surface_state_before = ?surface_state_before,
                                    surface_state_after = ?surface_state_after,
                                    "RenderThread resumed (dirty forced for first-frame paint)"
                                );
                            }
                        }

                        RenderCommand::SurfaceDestroyed => {
                            let surface_state_before = surface_system.state();
                            let paused_now = *paused;
                            mark_surface_destroyed(surface_system);
                            // R1: Java clears surfaceReady (removing any posted
                            // callback); clear our in-flight flag so a later
                            // RecreateOnscreen re-arms retained demand.
                            vsync_armed.set(false);
                            info!(
                                paused = paused_now,
                                surface_state_before = ?surface_state_before,
                                surface_state_after = ?surface_system.state(),
                                "RenderThread SurfaceDestroyed acknowledged"
                            );
                        }

                        RenderCommand::TrimTextCache { level } => {
                            // Text texture cache lives process-global but
                            // its GL textures can only be freed with a
                            // current EGL context, which only this thread
                            // has.  Trim, then delete the returned victim
                            // textures.
                            let lvl =
                                shared::text_texture_cache::TrimLevel::from_android(level);
                            let (victims, stats) = {
                                let mut tc = shared::text_texture_cache::global_cache();
                                let v = tc.trim(lvl);
                                (v, tc.stats())
                            };
                            if !victims.is_empty() && cm.ensure_any_canvas_current().is_ok() {
                                unsafe {
                                    for raw in &victims {
                                        if let Some(t) = <glow::NativeTexture as NativeTextureFromRawInline>::try_from_raw(*raw) {
                                            gl.delete_texture(t);
                                        }
                                    }
                                }
                            }
                            crate::render_diagnostics::set_text_cache_gauges(
                                stats.size_bytes as u32,
                                stats.entries as u32,
                            );
                            // Android memory pressure is an explicit cleanup
                            // edge even while the app stays foregrounded. Keep
                            // the existing age threshold; unlike Pause this
                            // does not lower the long-lived Skia cache budget.
                            cm.perform_deferred_cleanup_all(DEFERRED_CLEANUP_UNUSED_AGE);
                            cleanup_cadence.mark_forced(start_time.elapsed());
                            info!(
                                "RenderThread: text cache trim level={} freed_textures={} resident_bytes={}",
                                level,
                                victims.len(),
                                stats.size_bytes
                            );
                        }

                        RenderCommand::LoadFont { family, aliases, bytes, resp } => {
                            // F-2: the render thread + the JS-thread
                            // measurer share the same `TextContext` via
                            // `Arc<Mutex<_>>`, so a single registration
                            // is visible to both sides.  Locking only
                            // happens here (one-shot on font load);
                            // measurement contention is on the JS side
                            // which locks separately.
                            let mut text_ctx = renderer_2d.text.lock();
                            if let Some(registration) =
                                text_ctx.register_family_aliases(aliases.as_ref().as_slice(), &bytes)
                            {
                                // A newly-registered / replaced typeface
                                // can change how any cached text rendered:
                                // bump the global font generation so all
                                // existing text-texture-cache entries
                                // (keyed on the prior generation) become
                                // unreachable and age out via LRU.  New
                                // fillTexts capture the bumped generation.
                                let font_gen = shared::text_texture_cache::bump_font_generation();
                                info!(
                                    "RenderThread: loaded font '{}' aliases={:?} internal_family={:?} ({} bytes), text_cache_generation={}",
                                    family,
                                    registration.aliases,
                                    registration.internal_family,
                                    bytes.len(),
                                    font_gen
                                );
                                resp.ok(family);
                            } else {
                                warn!(
                                    "RenderThread: failed to parse font '{}' aliases={:?} ({} bytes)",
                                    family,
                                    aliases,
                                    bytes.len()
                                );
                                resp.err_msg(format!(
                                    "font '{family}' rejected: invalid typeface data"
                                ));
                            }
                        }

                        RenderCommand::GetTextLineHeight { font_family, font_size, bold, italic, resp } => {
                            use crate::backend::gl::state::TextAttrs;
                            use shared::protocol::render_cmd::{TextAlign, TextBaseline, TextDirection};
                            let attrs = TextAttrs {
                                size: font_size,
                                families: std::sync::Arc::new(vec![font_family, "sans-serif".into()]),
                                weight: if bold { 700 } else { 400 },
                                italic,
                                align: TextAlign::Start,
                                baseline: TextBaseline::Alphabetic,
                                direction: TextDirection::Inherit,
                            };
                            // Empty-string layout still reports line metrics.
                            let text_ctx = renderer_2d.text.lock();
                            let m = text_ctx.measure_text(" ", &attrs);
                            drop(text_ctx);
                            let line_height = m.font_bounding_box_ascent
                                + m.font_bounding_box_descent;
                            let result = if line_height > 0.0 {
                                line_height
                            } else {
                                font_size * 1.2
                            };
                            resp.ok(result);
                        }

                        _ => {}
                    }

                    LoopCtl::Continue
                };

                let drain_cmds = |cm: &mut CanvasManager,
                                      gl: &glow::Context,
                                      canvas_handler: &mut CanvasHandler,
                                      renderer_2d: &mut Renderer2d,
                                       renderer_gl: &mut RendererGL,
                                       fps: &mut u32,
                                       frame_scheduler: &mut FrameScheduler,
                                       ticker: &mut Receiver<Instant>,
                                       dirty: &mut bool,
                                       paused: &mut bool,
                                       surface_system: &mut SurfaceSystem,
                                       render_server: &mut RenderServer|
                 -> LoopCtl {
                    crate::atrace_scope!("migo.render.drain_cmds");
                    // Drain pending commands with a *dual budget*:
                    // capped by both command count and elapsed CPU
                    // time.  Previously only the count was bounded,
                    // which let a burst of cheap but numerous ops
                    // (thousands of scalar `bindBuffer` /
                    // `uniform*`) still add up to millions of
                    // nanoseconds before VSync caught up, turning
                    // into frame-time jitter rather than average
                    // throughput.
                    //
                    // MAX_CMDS keeps us from starving the outer
                    // select! loop; MAX_DRAIN_US caps the worst-
                    // case CPU burst so a backlog spills across
                    // frames instead of pathologically stretching
                    // one.  Values are conservative -- profile will
                    // tell us whether to widen / narrow.
                    const MAX_CMDS: usize = 512;
                    const MAX_DRAIN_US: u128 = 1_500; // 1.5 ms
                    let drain_start = Instant::now();
                    // P1-9: track whether the loop exited because of
                    // the CPU budget vs naturally.  Budget exits
                    // bump a counter so the overlay can distinguish
                    // "engine busy" from "engine idle".
                    let mut budget_exit = false;
                    for i in 0..MAX_CMDS {
                        // Re-check the CPU budget every 32 commands
                        // rather than on every iteration; the loop
                        // body is ~100 ns for a cheap cmd so the
                        // query cadence doesn't need to be tighter.
                        if i > 0 && (i & 31) == 0
                            && drain_start.elapsed().as_micros() >= MAX_DRAIN_US
                        {
                            budget_exit = true;
                            break;
                        }
                        match cmd_rx.try_recv() {
                            Ok(cmd) => {
                                match handle_one_cmd(cmd, cm, gl, canvas_handler, renderer_2d, renderer_gl, fps, frame_scheduler, ticker, dirty, paused, has_vsync, surface_system, render_server) {
                                    LoopCtl::Continue => {}
                                    LoopCtl::Shutdown => return LoopCtl::Shutdown,
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    if budget_exit {
                        debug_stats
                            .drain_budget_exhausted
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    LoopCtl::Continue
                };

                // Shared frame presentation + RAF signal logic.
                // Free-run RAF signal. Writes the frame timestamp with the
                // CONSTANT per-session ticket, and is called every scheduled
                // vsync while animating — NOT gated on a live waiter. That keeps
                // the eventfd primed so the JS consumer's `recv` returns without
                // blocking, so the compute thread stays busy and the CPU governor
                // holds a high clock. (The old demand-latch path signalled only
                // when JS was already awaiting, so JS blocked once per frame, its
                // utilisation fell to ~40%, the governor parked it at ~1.5GHz, and
                // stress fps halved.) Returns whether the signal was delivered.
                let signal_raf = |paused: bool, ts: f64| -> bool {
                    if paused {
                        return false;
                    }
                    let ticket = raf_demand.session_ticket();
                    if raf_tx.signal(ts, ticket) {
                        let prev_streak =
                            debug_stats.raf_drop_streak.swap(0, Ordering::Relaxed);
                        if prev_streak >= RAF_BACKPRESSURE_STREAK_THRESHOLD {
                            info!(prev_streak, "RAF backpressure streak cleared");
                        }
                        true
                    } else {
                        debug_stats.dropped_frames.fetch_add(1, Ordering::Relaxed);
                        let streak =
                            debug_stats.raf_drop_streak.fetch_add(1, Ordering::Relaxed) + 1;
                        if streak == RAF_BACKPRESSURE_STREAK_THRESHOLD {
                            debug_stats
                                .raf_backpressure_events
                                .fetch_add(1, Ordering::Relaxed);
                            events.emit(RenderEvent::RafBackpressure {
                                consecutive_drops: streak,
                            });
                            info!(
                                consecutive_drops = streak,
                                threshold = RAF_BACKPRESSURE_STREAK_THRESHOLD,
                                "RAF backpressure threshold crossed"
                            );
                        }
                        false
                    }
                };

                let present_frame_and_signal_raf = |cm: &mut CanvasManager,
                                                         renderer_2d: &mut Renderer2d,
                                                         dirty: &mut bool,
                                                         paused: bool,
                                                         should_present: bool,
                                                         ts: f64,
                                                         debug_stats: &shared::stats::DebugStats,
                                                         frame_count: &mut u32,
                                                        fps_timer: &mut Instant,
                                                        last_frame_time: &mut Instant,
                                                        first_frame_recorded: &mut bool,
                                                        needs_recovery: &mut bool| {
                    crate::atrace_scope!("migo.render.present_and_raf");
                    let _ = ts; // RAF is signalled before the drain now (see signal_raf)
                    // Drain Canvas2D snapshot textures captured during this
                    // frame's `getImageData` calls.  By this point the
                    // FramePacket has already executed every queued
                    // `TexImage2DFromSnapshot`, so the snapshots are no
                    // longer referenced by any pending command.  Deleting
                    // them now keeps the pool tiny under the cocos text-
                    // rendering pattern (hundreds of getImageData calls per
                    // frame).
                    cm.drain_canvas2d_snapshots();
                    // Drain completed texture uploads from the upload thread
                    // and register them in the image registry for rendering.
                    let dropped_recoveries = cm.drain_upload_completed();
                    if dropped_recoveries > 0 {
                        debug_stats.dropped_upload_recoveries.fetch_add(dropped_recoveries, Ordering::Relaxed);
                    }
                    // Capture upload rejections before reset clears the counter.
                    let rejections = cm.take_upload_frame_rejections();
                    if rejections > 0 {
                        debug_stats.upload_frame_rejections.fetch_add(rejections, Ordering::Relaxed);
                    }
                    // Reset per-frame upload budget for the new frame.
                    cm.reset_frame_upload_budget();
                    // Retry uploads that the previous frame's budget
                    // rejected.  Draining before the frame's fresh
                    // draw commands means the backlog claims budget
                    // first; the budget is monotonic within a frame,
                    // so a rejection stops the loop and the rest
                    // waits for the next frame's window.
                    cm.try_drain_deferred_uploads();
                    // RAF was already signalled before the drain (see `signal_raf`),
                    // so JS is producing the next frame in parallel with this swap.

                    // R-3: robustness poll — if the driver flagged a
                    // GL reset between frames, short-circuit the
                    // present path and let the next-frame recovery
                    // handle teardown.  Without this poll, a silently
                    // reset context would keep emitting "successful"
                    // GL calls (all no-ops on the wrong state) until
                    // `eglSwapBuffers` eventually caught it — meaning
                    // an entire frame's worth of Canvas2D / WebGL
                    // work gets run against a dead context before the
                    // user sees the black screen.
                    if cm.check_graphics_reset_status() {
                        *needs_recovery = true;
                        debug_stats
                            .context_lost_events
                            .fetch_add(1, Ordering::Relaxed);
                        let failed = cm.fail_pending_sync_responders(
                            "GL_KHR_robustness reported context reset",
                        );
                        if failed > 0 {
                            warn!("Aborted {failed} pending sync responder(s) due to robustness-reported reset");
                        }
                        cm.abandon_all_2d_contexts();
                        // Edge-triggered authoritative write (single packed
                        // atomic): `set_lost` returns true only on the
                        // false->true transition, so we emit + notify exactly
                        // once. A sustained / re-detected loss must not keep
                        // flooding the host with duplicate events.
                        if context_lost.set_lost() {
                            events.emit(RenderEvent::ContextLost);
                        }
                    }

                    // Present the completed frame (only if we have a valid surface).
                    // Re-check the surface epoch here, immediately before the swap:
                    // the caller's gate ran before upload-drain / flush / RAF work,
                    // and a surfaceDestroyed may have advanced the epoch since. This
                    // closes the residual "check early, swap late" window.
                    let surface_current =
                        destroy_epoch.load(std::sync::atomic::Ordering::Acquire) == valid_epoch.get();
                    let did_swap = if *dirty && should_present && surface_current {
                        let onscreen_id = shared::protocol::render_cmd::CanvasId::from(1u32);
                        let (canvas_w, canvas_h) = cm.get_canvas_size(onscreen_id).unwrap_or((0, 0));
                        let tracked_viewport = cm
                            .gl_state
                            .get(&onscreen_id)
                            .and_then(|s| s.viewport)
                            .unwrap_or((0, 0, canvas_w as i32, canvas_h as i32));
                        // Declare damage region BEFORE any onscreen rendering
                        // (Skia flush_and_submit, DrawingBuffer blit). Per EGL_KHR_partial_update
                        // spec, the declaration must precede GL draws to the main
                        // framebuffer so the driver can skip loading unchanged tiles.
                        cm.declare_frame_damage(onscreen_id);

                        crate::atrace_scope!("migo.render.flush_2d");
                        match cm.flush_dirty_2d_contexts() {
                            Ok(flushed_ids) => {
                                for canvas_id in flushed_ids {
                                    renderer_2d.clear_dirty_layer(canvas_id);
                                }
                            }
                            Err(e) => {
                                warn!("flush_dirty_2d_contexts failed: {}", e);
                            }
                        }
                        unsafe {
                            gl.viewport(
                                tracked_viewport.0,
                                tracked_viewport.1,
                                tracked_viewport.2,
                                tracked_viewport.3,
                            )
                        };

                        crate::atrace_scope!("migo.render.swap_buffers");
                        // Final epoch re-check at the swap boundary: a surfaceDestroyed
                        // may have raced the flush/damage work above. If stale, skip the
                        // EGL swap entirely (the next present-gate iteration marks the
                        // surface lost); presenting to the abandoned BufferQueue is the
                        // one thing we must never do.
                        let swap_ok = if destroy_epoch.load(std::sync::atomic::Ordering::Acquire)
                            != valid_epoch.get()
                        {
                            false
                        } else {
                            match cm.swap_buffers_no_restore(shared::protocol::render_cmd::CanvasId::from(1u32), true) {
                            Ok(resolved_damage) => {
                                use crate::dirty_region::damage_tracker::ResolvedDamage;
                                match resolved_damage {
                                    ResolvedDamage::Partial { width, height, .. } => {
                                        debug_stats.partial_damage_frames.fetch_add(1, Ordering::Relaxed);
                                        let kpx = ((width as u64) * (height as u64) / 1000) as u32;
                                        debug_stats.damage_area_k_pixels.fetch_add(kpx, Ordering::Relaxed);
                                    }
                                    ResolvedDamage::FullSurface => {
                                        debug_stats.full_surface_frames.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                true
                            }
                            Err(e) => {
                                warn!("swap_buffers_no_restore failed: {}", e);
                                if cm.is_context_lost() {
                                    *needs_recovery = true;
                                    warn!("EGL context lost, recovery deferred to next frame");
                                    debug_stats
                                        .context_lost_events
                                        .fetch_add(1, Ordering::Relaxed);
                                    // P1-2: fail any pending sync op the
                                    // manager is still holding on behalf of
                                    // a JS caller.  Prevents 10 s
                                    // COMMAND_TIMEOUT_MS stalls on
                                    // Image.src / getImageData when EGL
                                    // state has already been discarded.
                                    let failed = cm.fail_pending_sync_responders(
                                        "EGL context lost; responder aborted before recovery",
                                    );
                                    if failed > 0 {
                                        warn!(
                                            "Aborted {failed} pending sync responder(s) due to context loss"
                                        );
                                    }
                                    // R-2: mark every Skia context
                                    // abandoned so `Drop` can't try
                                    // `glDelete*` against a dead EGL
                                    // context.  `try_recover_context`
                                    // on the next frame will recreate
                                    // fresh `Canvas2DContext`s.
                                    cm.abandon_all_2d_contexts();
                                    // Edge-triggered: emit + notify only on the
                                    // false->true transition (see robustness path).
                                    if context_lost.set_lost() {
                                        events.emit(RenderEvent::ContextLost);
                                    }
                                } else {
                                    debug_stats
                                        .swap_failures
                                        .fetch_add(1, Ordering::Relaxed);
                                    events.emit(RenderEvent::SwapFailed {
                                        message: e.to_string(),
                                    });
                                }
                                false
                            }
                            }
                        };
                        *dirty = false;

                        if swap_ok && !*first_frame_recorded {
                            *first_frame_recorded = true;
                            let first_ms = start_time.elapsed().as_millis() as u32;
                            debug_stats.first_frame_ms.store(first_ms, Ordering::Relaxed);
                        }
                        swap_ok
                    } else {
                        false
                    };

                    // v4 cache / queue observability: sampled once
                    // per frame; the overlay reads them via
                    // `DebugStats::snapshot()`.  Placed after the
                    // swap so wrappers created during the frame's
                    // flush are reflected immediately.
                    crate::render_diagnostics::set_sk_image_wrapper_count(
                        cm.image_wrapper_cache_len() as u32,
                    );
                    crate::render_diagnostics::set_deferred_uploads(
                        cm.deferred_uploads_len() as u32,
                    );

                    // Piggyback maintenance on an already-requested physical
                    // frame. No timer is armed while idle, and advancing to
                    // `now` prevents catch-up sweeps after a long pause.
                    if cleanup_cadence.should_run(start_time.elapsed()) {
                        cm.perform_deferred_cleanup_all(DEFERRED_CLEANUP_UNUSED_AGE);
                    }

                    // FPS stats.
                    let now = Instant::now();
                    if did_swap {
                        let frame_dur = now.duration_since(*last_frame_time);
                        *last_frame_time = now;
                        debug_stats.frame_time_us.store(frame_dur.as_micros() as u32, Ordering::Relaxed);
                        *frame_count += 1;
                    }
                    let elapsed = fps_timer.elapsed();
                    if elapsed >= Duration::from_millis(500) {
                        let measured_fps = *frame_count as f32 / elapsed.as_secs_f32();
                        debug_stats.fps_x10.store((measured_fps * 10.0) as u32, Ordering::Relaxed);
                        *frame_count = 0;
                        *fps_timer = now;
                    }
                    crate::render_diagnostics::flush_frame();
                };

                // R1: request exactly one more display frame iff demand remains
                // (RAF waiter / dirty / outstanding upload work) and we can present.
                // Idle, paused, or surfaceless => no arm, so the frame clock stops
                // and there is no idle JNI flood. `vsync_armed` suppresses a
                // redundant request while one callback is already in flight; Java's
                // own requested/callbackPosted latch is the authoritative dedup. A
                // no-op on the software-ticker path (`has_vsync` false / no closure).
                let arm_if_needed =
                    |warm: bool,
                     paused: bool,
                     dirty: bool,
                     recovery_pending: bool,
                     cm: &CanvasManager,
                     surface_system: &SurfaceSystem| {
                        // `warm`: an actively-animating loop this frame. Keep the
                        // vsync clock armed continuously (rather than re-deriving
                        // demand from `is_waiting()`, which the free-run signal has
                        // just consumed) so vsyncs stay at display rate; the warm
                        // window drains to false a few frames after JS goes idle,
                        // then the clock stops.
                        let demand = warm
                            || crate::frame_scheduler::raf_demand_remains(
                                raf_demand.is_waiting(),
                                dirty,
                                cm.has_outstanding_upload_work(),
                                recovery_pending,
                            );
                        if crate::frame_scheduler::should_arm_one_shot(
                            has_vsync,
                            request_vsync.is_some(),
                            paused,
                            surface_system.can_present(),
                            demand,
                            vsync_armed.get(),
                        ) {
                            if let Some(rv) = request_vsync.as_ref() {
                                rv();
                            }
                            vsync_armed.set(true);
                        }
                    };

                // Free-run warm window: number of scheduled vsyncs to keep
                // signalling RAF and arming the clock after the last frame in
                // which JS was observed awaiting. Non-zero == "animating". A
                // consumed waiter refills it; it decays to 0 a few frames after JS
                // stops requesting, which stops the clock (idle power preserved).
                const RAF_WARM_FRAMES: u8 = 3;
                let mut warm_frames: u8 = 0;

                loop {
                    // Shutdown has an out-of-band level because the bounded
                    // command queue deliberately drops lifecycle commands when
                    // full. The best-effort Shutdown command wakes an idle loop;
                    // this level guarantees exit after a saturated queue drains.
                    if shutdown_requested.load(Ordering::Acquire) {
                        info!("RenderThread observed shutdown request");
                        cm.destroy_all(&gl);
                        shared::stats::unregister_stats(host_id);
                        return;
                    }

                    // --- Deferred EGL context recovery ---
                    // Performed at the top of the frame loop where it is less
                    // timing-critical than inside the swap path. This avoids
                    // blocking the RAF signal with a full EGL teardown+recreate.
                    if needs_context_recovery {
                        needs_context_recovery = false;
                        match cm.try_recover_context() {
                            Ok(true) => {
                                info!("EGL context recovered at frame top, resuming rendering");
                                // The rebuilt contexts have no presented contents.
                                // Force one repaint and keep the already-requested
                                // one-shot alive so a static scene cannot remain
                                // frozen after recovery.
                                dirty = true;
                                debug_stats
                                    .context_recoveries
                                    .fetch_add(1, Ordering::Relaxed);
                                // Edge-triggered clear (single packed atomic):
                                // `set_recovered` returns true only on the
                                // true->false transition, atomically publishing
                                // the new level+epoch so a `webglcontextrestored`
                                // listener sees isContextLost()==false.
                                if context_lost.set_recovered() {
                                    events.emit(RenderEvent::ContextRecovered {
                                        success: true,
                                    });
                                }
                            }
                            Ok(false) => {
                                // Not recovered: either no window handle yet, or
                                // the rebuilt context failed its probe draw and
                                // is still unusable. The context remains lost, so
                                // report an honest failure (isContextLost() stays
                                // true) rather than a false "recovered".
                                if report_recovery_failure(
                                    &context_lost,
                                    &events,
                                    &mut last_recovery_fail_epoch,
                                ) {
                                    warn!("EGL context recovery incomplete; context still lost");
                                }
                                needs_context_recovery = true;
                            }
                            Err(re) => {
                                if report_recovery_failure(
                                    &context_lost,
                                    &events,
                                    &mut last_recovery_fail_epoch,
                                ) {
                                    warn!("EGL context recovery failed: {}", re);
                                }
                                needs_context_recovery = true;
                            }
                        }
                    }

                    select! {
                        recv(ticker) -> _ => {
                            // Software ticker path (non-Android fallback).
                            // Frame timing: drain -> swap -> RAF signal.
                            crate::render_diagnostics::set_render_queue_len(cmd_rx.len() as u32);
                            let ts = start_time.elapsed().as_secs_f64() * 1000.0;
                            render_server.set_raf_time_ms(ts);

                            // 1) Signal RAF first (free-run), unconditionally while
                            // animating, so JS produces the next frame in parallel
                            // with this drain+swap and never blocks between frames.
                            let has_waiter = raf_demand.take_waiter().is_some();
                            if has_waiter {
                                warm_frames = RAF_WARM_FRAMES;
                            } else if warm_frames > 0 {
                                warm_frames -= 1;
                            }
                            if has_waiter || warm_frames > 0 {
                                let _ = signal_raf(paused, ts);
                            }

                            // 2) Drain all pending commands from the previous frame.
                            match drain_cmds(&mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut frame_scheduler, &mut ticker, &mut dirty, &mut paused, &mut surface_system, &mut render_server) {
                                LoopCtl::Continue => {}
                                LoopCtl::Shutdown => {
                                    shared::stats::unregister_stats(host_id);
                                    return;
                                }
                            }

                            // 3) Present (swap) the drained frame.
                            // If a surfaceDestroyed has occurred since our current
                            // surface was handed off (live epoch moved past it),
                            // stop presenting to it now — before Java returns and
                            // Android abandons the BufferQueue — without waiting
                            // for the async SurfaceDestroyed command.
                            if destroy_epoch.load(std::sync::atomic::Ordering::Acquire) != valid_epoch.get() {
                                surface_system.on_surface_destroyed();
                            }
                            present_frame_and_signal_raf(&mut cm, &mut renderer_2d, &mut dirty, paused, surface_system.can_present(), ts, &debug_stats, &mut frame_count, &mut fps_timer, &mut last_frame_time, &mut first_frame_recorded, &mut needs_context_recovery);
                            // No-op on the software-ticker path (has_vsync false);
                            // kept for symmetry with the vsync branch.
                            arm_if_needed(false, paused, dirty, needs_context_recovery, &cm, &surface_system);
                        }

                        recv(vsync) -> _msg => {
                            // Choreographer VSync path (Android).
                            // R1: the requested one-shot callback was delivered —
                            // clear the in-flight flag so demand that remains this
                            // frame can re-arm the next one.
                            vsync_armed.set(false);
                            crate::render_diagnostics::set_render_queue_len(cmd_rx.len() as u32);
                            let Some(frame_time_ms) = _msg.ok() else {
                                continue;
                            };

                            let decision = next_vsync_frame_decision(
                                &mut frame_scheduler,
                                &surface_system,
                                frame_time_ms,
                            );

                            if !decision.should_signal_raf {
                                // RAF-skipped tick (e.g. game hasn't
                                // called requestAnimationFrame yet
                                // during startup).  We still need to
                                // open the per-frame upload-budget
                                // window and retry any uploads that
                                // the previous tick deferred --
                                // otherwise burst image loads at
                                // boot sit in `deferred_uploads`
                                // forever while the budget never
                                // resets, surfacing as a 10 s
                                // OP_LOAD_IMAGE timeout in JS.
                                cm.reset_frame_upload_budget();
                                cm.try_drain_deferred_uploads();
                                // Also pick up any completed fences
                                // so resp oneshots from prior ticks
                                // don't sit idle either.
                                let _ = cm.drain_upload_completed();
                                // A scheduler-skipped physical vsync still has
                                // pending demand (a RAF waiter awaiting the next
                                // target-FPS deadline on a 90/120Hz panel, or
                                // outstanding upload work) — re-arm so we keep
                                // receiving physical vsyncs until the deadline.
                                arm_if_needed(warm_frames > 0, paused, dirty, needs_context_recovery, &cm, &surface_system);
                                continue;
                            }

                            render_server.set_raf_time_ms(decision.raf_time_ms);

                            // 1) Signal RAF FIRST, and unconditionally while
                            // animating (free-run). A consumed waiter marks JS as
                            // actively animating and refills the warm window; while
                            // warm we keep signalling even on frames where JS has
                            // not yet re-published demand, so its `recv` finds a
                            // primed signal and never blocks — keeping the compute
                            // thread busy (high clock). The warm window decays a few
                            // frames after JS stops, then the clock idles.
                            let has_waiter = raf_demand.take_waiter().is_some();
                            if has_waiter {
                                warm_frames = RAF_WARM_FRAMES;
                            } else if warm_frames > 0 {
                                warm_frames -= 1;
                            }
                            let animating = has_waiter || warm_frames > 0;
                            if animating {
                                let _ = signal_raf(paused, decision.raf_time_ms);
                            }

                            // 2) Drain all pending commands (in parallel with JS
                            // producing the next frame, which the signal above woke).
                            match drain_cmds(&mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut frame_scheduler, &mut ticker, &mut dirty, &mut paused, &mut surface_system, &mut render_server) {
                                LoopCtl::Continue => {}
                                LoopCtl::Shutdown => {
                                    shared::stats::unregister_stats(host_id);
                                    return;
                                }
                            }

                            // 3) Present (swap) the drained frame.
                            if destroy_epoch.load(std::sync::atomic::Ordering::Acquire) != valid_epoch.get() {
                                surface_system.on_surface_destroyed();
                            }
                            let should_present = decision.should_signal_raf && surface_system.can_present();
                            present_frame_and_signal_raf(&mut cm, &mut renderer_2d, &mut dirty, paused, should_present, decision.raf_time_ms, &debug_stats, &mut frame_count, &mut fps_timer, &mut last_frame_time, &mut first_frame_recorded, &mut needs_context_recovery);
                            // Keep the vsync clock at display rate while animating;
                            // otherwise re-arm only on residual demand (dirty /
                            // upload), else the clock stops.
                            arm_if_needed(animating, paused, dirty, needs_context_recovery, &cm, &surface_system);
                        }

                        recv(cmd_rx) -> msg => {
                            match msg {
                                Ok(cmd) => {
                                    match handle_one_cmd(cmd, &mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut frame_scheduler, &mut ticker, &mut dirty, &mut paused, has_vsync, &mut surface_system, &mut render_server) {
                                        LoopCtl::Continue => {}
                                        LoopCtl::Shutdown => {
                                            shared::stats::unregister_stats(host_id);
                                            return;
                                        }
                                    }
                                    // Drain remaining pending commands.
                                    match drain_cmds(&mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut frame_scheduler, &mut ticker, &mut dirty, &mut paused, &mut surface_system, &mut render_server) {
                                        LoopCtl::Continue => {}
                                        LoopCtl::Shutdown => {
                                            shared::stats::unregister_stats(host_id);
                                            return;
                                        }
                                    }
                                    // Eager upload drain: LoadImage ops
                                    // submit work to the upload thread
                                    // immediately, but `drain_upload_
                                    // completed` used to run only at
                                    // the next VSync tick.  With a
                                    // 60 Hz refresh that's up to 16 ms
                                    // between "fence signalled" and
                                    // "Image.onload fires in JS".
                                    // Running a non-blocking drain here
                                    // -- right after the JS-facing
                                    // command batch -- drops the median
                                    // latency to sub-millisecond on
                                    // mobile tiled GPUs that coalesce
                                    // the upload+fence pair quickly.
                                    // The drain is a bounded cost: it
                                    // only does `glClientWaitSync(...,
                                    // timeout=0)` per pending entry,
                                    // returning immediately whether the
                                    // fence is ready or not.
                                    let dropped_recoveries = cm.drain_upload_completed();
                                    if dropped_recoveries > 0 {
                                        debug_stats.dropped_upload_recoveries.fetch_add(dropped_recoveries, Ordering::Relaxed);
                                    }
                                    // R1: a command batch may have created demand
                                    // (dirty onscreen content, or a LoadImage that
                                    // is now in-flight) — arm one frame to present
                                    // / poll the fence. Idle commands don't arm.
                                    arm_if_needed(false, paused, dirty, needs_context_recovery, &cm, &surface_system);
                                }
                                Err(_) => {
                                    info!("Command channel closed, exiting RenderThread");
                                    cm.destroy_all(&gl);
                                    shared::stats::unregister_stats(host_id);
                                    return;
                                }
                            }
                        }
                    }
                }
                })); // end catch_unwind
                if let Err(panic_info) = result {
                    let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                        s.clone()
                    } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                        s.to_string()
                    } else {
                        "Unknown panic".to_string()
                    };
                    if !gpu_caps.is_ready() {
                        gpu_caps.set_failed(format!(
                            "RenderThread panicked during initialization: {msg}"
                        ));
                    }
                    error!("[RenderThread host={}] PANIC: {}", host_id, msg);
                    if let Some(stats) = shared::stats::get_stats(host_id) {
                        stats.fatal_error_code.store(
                            shared::error::ErrorCode::Internal.as_u16() as u32,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                    }
                    shared::stats::unregister_stats(host_id);
                }
            })
            .map_err(|e| {
                EngineError::from_detail(
                    ErrorCode::IoError,
                    format!("failed to spawn render thread: {}", e),
                )
            })?;

        Ok(Self {
            cmd_tx,
            shutdown_requested: shutdown_requested_for_owner,
            event_rx,
            text_measurer,
            handle: Some(handle),
        })
    }

    pub fn sender(&self) -> CommandSender {
        self.cmd_tx.clone()
    }

    /// Clone the event feedback receiver.  Callers poll /
    /// `try_recv` this to surface render-thread failures into JS
    /// (error events, `performance.mark`) or into the Android
    /// debug overlay.  Multiple consumers may `clone()` the
    /// receiver — events are delivered to whichever consumer
    /// wins the underlying crossbeam MPMC race, so deploying more
    /// than one subscriber only makes sense if the consumers
    /// partition on event `kind()`.
    pub fn events(&self) -> RenderEventReceiver {
        self.event_rx.clone()
    }

    /// F-2: clone the shared `TextMeasurer` handle.  Host runtime
    /// wiring installs this on `CanvasOpState` so JS-side
    /// `op_measure_text_flat` / `op_get_text_line_height` can
    /// bypass the render thread entirely for the measurement
    /// fast path.
    pub fn text_measurer(&self) -> shared::text_measurer::SharedTextMeasurer {
        self.text_measurer.clone()
    }

    pub fn shutdown(&mut self) {
        let Some(h) = self.handle.take() else {
            return;
        };
        self.shutdown_requested.store(true, Ordering::Release);
        let _ = self.cmd_tx.send(RenderCommand::Shutdown);
        let _ = h.join();
    }

    pub fn shutdown_detached(&mut self) {
        let Some(h) = self.handle.take() else {
            return;
        };
        self.shutdown_requested.store(true, Ordering::Release);
        let _ = self.cmd_tx.send(RenderCommand::Shutdown);
        // This path is also called from Drop while unwinding. Builder::spawn
        // returns an error instead of panicking if a joiner cannot be created;
        // dropping `h` then detaches it after Shutdown was already sent.
        let _ = std::thread::Builder::new()
            .name("Migo-RenderJoin".into())
            .spawn(move || {
                let _ = h.join();
            });
    }
}
