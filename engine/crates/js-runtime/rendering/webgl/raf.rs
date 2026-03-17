use std::{cell::RefCell, rc::Rc};

use deno_core::{OpState, op2};
use shared::op_state::HostOpState;
use shared::protocol::render_cmd::RenderCommand;

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
#[op2(async(lazy), fast)]
pub(crate) async fn op_await_next_frame(state: Rc<RefCell<OpState>>) -> Result<f64, RafError> {
    let rx = {
        let st = state.borrow();
        st.borrow::<HostOpState>()
            .raf_rx
            .clone()
            .ok_or_else(|| RafError::Message("RAF receiver not initialized".into()))?
    };
    let mut guard = rx.lock().await;
    guard
        .recv()
        .await
        .ok_or_else(|| RafError::Message("RAF channel closed".into()))
}

/// Sync op to set the preferred frame rate (1–60 fps).
///
/// On Choreographer: adjusts the frame divisor (skip VSync signals).
/// On software ticker: recreates the ticker at the new interval.
#[op2(fast)]
pub(crate) fn op_set_preferred_fps(state: &mut OpState, #[smi] fps: u32) {
    let fps = fps.clamp(1, 60);
    let tx = state.borrow::<HostOpState>().render_tx.clone();
    let _ = tx.send(RenderCommand::FrameRate(fps));
}
