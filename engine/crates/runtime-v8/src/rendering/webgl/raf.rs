use std::{
    cell::RefCell,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

use deno_core::{OpState, op2};
use shared::op_state::HostOpState;
use shared::protocol::render_cmd::RenderCommand;

static RAF_WAIT_LOGGED: AtomicBool = AtomicBool::new(false);
static RAF_RECV_LOGGED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, thiserror::Error, deno_error::JsError)]
pub enum RafError {
    #[class("RafError")]
    #[error("{0}")]
    Message(String),
}

/// Async op that blocks until the next frame signal arrives from the render thread.
///
/// Returns the frame timestamp in milliseconds. The pending future keeps the
/// deno_core event loop alive, which eliminates the busy-loop problem.
///
/// # Why the tokio::Mutex on the mpsc::Receiver
///
/// `tokio::sync::mpsc::Receiver` is single-consumer and only one JS task ever
/// calls this op at a time (the RAF polling loop).  However, the Mutex is still
/// architecturally required for two reasons:
///
/// 1. **OpState is `Rc<RefCell>`, not `Send`.**  The receiver must be extracted
///    from OpState and held across an `.await` point (`recv().await`).  Wrapping
///    it in `Arc<tokio::sync::Mutex>` makes the future `Send`-compatible, which
///    deno_core requires for async ops even on a single-threaded runtime.
///
/// 2. **Restart survival.**  The `RafRx` is shared between the Host and OpState
///    so it survives JS runtime restarts (Host keeps an `Arc` clone).  The Mutex
///    ensures the handoff is safe if the old runtime's in-flight future races
///    with the new runtime's first call during a restart.
///
/// In practice there is no contention: only one task holds the lock at a time,
/// so the Mutex acquisition is uncontested and effectively free.
#[op2(async(lazy), fast)]
pub(crate) async fn op_await_next_frame(state: Rc<RefCell<OpState>>) -> Result<f64, RafError> {
    let (rx, host_id, demand, arm) = {
        let st = state.borrow();
        let host_state = st.borrow::<HostOpState>();
        (
            host_state
                .raf_rx
                .clone()
                .ok_or_else(|| RafError::Message("RAF receiver not initialized".into()))?,
            host_state.id,
            host_state.raf_demand.clone(),
            host_state.request_vsync.clone(),
        )
    };

    // R1: publish demand and kick the one-shot arm BEFORE awaiting, so an idle
    // display clock wakes up and the render thread knows a waiter is pending
    // (it only signals RAF when the demand latch is set).
    let ticket = raf_publish_demand_and_arm(&demand, arm.as_ref());

    if !RAF_WAIT_LOGGED.swap(true, Ordering::Relaxed) {
        tracing::info!("op_await_next_frame first call: host={}", host_id);
    }

    let ts = rx
        .recv(ticket)
        .await
        .ok_or_else(|| RafError::Message("RAF channel closed".into()))?;

    if !RAF_RECV_LOGGED.swap(true, Ordering::Relaxed) {
        tracing::info!(
            "op_await_next_frame first frame received: host={}, ts_ms={:.3}",
            host_id,
            ts
        );
    }

    Ok(ts)
}

/// Publish RAF demand and kick the one-shot vsync arm — the pre-await half of
/// [`op_await_next_frame`], extracted so the ordering (publish demand, then arm)
/// is host-testable without a live tokio runtime. Marks the demand latch so the
/// render thread signals RAF for this waiter, then invokes the arm route (if
/// any) to wake an idle display clock. The arm is idempotent on the Java side.
pub(crate) fn raf_publish_demand_and_arm(
    demand: &shared::raf_signal::RafDemand,
    arm: Option<&std::sync::Arc<dyn Fn() + Send + Sync>>,
) -> u64 {
    let ticket = demand.mark_waiting();
    if let Some(arm) = arm {
        arm();
    }
    ticket
}

/// Sync op to set the preferred frame rate (1-120 fps).
///
/// On Choreographer: adjusts the frame divisor (skip VSync signals).
/// Engine-paced: widens or narrows the frame clock's pacing grid.
#[op2(fast)]
pub(crate) fn op_set_preferred_fps(state: &mut OpState, #[smi] fps: u32) {
    let tx = state.borrow::<HostOpState>().render_tx.clone();
    let _ = tx.send(RenderCommand::FrameRate(fps.clamp(1, 120)));
}

#[cfg(test)]
mod tests {
    use super::raf_publish_demand_and_arm;
    use shared::raf_signal::RafDemand;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn publishes_demand_and_kicks_arm_before_blocking() {
        let demand = RafDemand::new();
        let armed = Arc::new(AtomicUsize::new(0));
        let arm_flag = armed.clone();
        let arm: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            arm_flag.fetch_add(1, Ordering::Relaxed);
        });
        let ticket = raf_publish_demand_and_arm(&demand, Some(&arm));
        assert!(demand.is_waiting(), "demand published before awaiting");
        assert_ne!(ticket, 0, "zero is reserved for no waiter");
        assert_eq!(armed.load(Ordering::Relaxed), 1, "armed exactly once");
    }

    #[test]
    fn no_arm_closure_still_publishes_demand() {
        let demand = RafDemand::new();
        let ticket = raf_publish_demand_and_arm(&demand, None);
        assert!(
            demand.is_waiting(),
            "demand published even without an arm route"
        );
        assert_ne!(ticket, 0);
    }

    /// R1 source contract for the demand-driven RAF loop. The loop must stop the
    /// instant no callbacks remain (no idle-frame tail), and the resume hook must
    /// not restart (and thus arm a vsync) when the callback queue is empty.
    #[test]
    fn raf_loop_is_demand_driven_no_idle_tail() {
        let src = include_str!("03_raf.js");
        assert!(
            !src.contains("MAX_IDLE_FRAMES"),
            "idle-frame tail constant must be removed"
        );
        assert!(
            !src.contains("idleFrames"),
            "idle-frame counter must be removed"
        );
        assert!(
            src.contains("Object.keys(__raf_callbacks).length > 0"),
            "__migo_restart_raf_loop must only restart when a callback is queued"
        );
    }
}
