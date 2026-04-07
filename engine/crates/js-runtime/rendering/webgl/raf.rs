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
    let (rx, host_id) = {
        let st = state.borrow();
        let host_state = st.borrow::<HostOpState>();
        (
            host_state
                .raf_rx
                .clone()
                .ok_or_else(|| RafError::Message("RAF receiver not initialized".into()))?,
            host_state.id,
        )
    };

    if !RAF_WAIT_LOGGED.swap(true, Ordering::Relaxed) {
        tracing::info!("op_await_next_frame first call: host={}", host_id);
    }

    let ts = rx
        .recv()
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

/// Sync op to set the preferred frame rate (1-120 fps).
///
/// On Choreographer: adjusts the frame divisor (skip VSync signals).
/// On software ticker: recreates the ticker at the new interval.
#[op2(fast)]
pub(crate) fn op_set_preferred_fps(state: &mut OpState, #[smi] fps: u32) {
    let tx = state.borrow::<HostOpState>().render_tx.clone();
    let _ = tx.send(RenderCommand::FrameRate(fps.clamp(1, 120)));
}
