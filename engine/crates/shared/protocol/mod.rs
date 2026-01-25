use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded};
use tokio::{
    sync::{mpsc::UnboundedSender, oneshot},
    time::timeout,
};
use tracing::error;

use crate::{
    error::{EngineError, ErrorCode},
    op_state::CanvasOpState,
};

pub mod audio_cmd;
pub mod host_cmd;
pub mod io_cmd;
pub mod render_cmd;

use self::{
    io_cmd::IOCmd,
    render_cmd::{GLCmd, RenderCmdResp, RenderCommand},
};

const OP_GL: &str = "gl command";
const OP_FS_SYNC: &str = "fs sync command";
const OP_FS_ASYNC: &str = "fs async command";

/// Default timeout: 10 seconds.
static COMMAND_TIMEOUT_MS: AtomicU64 = AtomicU64::new(10_000);

#[inline]
pub fn command_timeout() -> Duration {
    Duration::from_millis(COMMAND_TIMEOUT_MS.load(Ordering::Relaxed))
}

/// Allows caller to tune command timeout globally.
pub fn set_command_timeout(dur: Duration) {
    // Avoid 0ms timeout that can cause flakiness.
    let ms = dur.as_millis().max(1) as u64;
    COMMAND_TIMEOUT_MS.store(ms, Ordering::Relaxed);
}

#[inline(always)]
fn timeout_err(op: &'static str, to: Duration) -> EngineError {
    EngineError::from_detail(
        ErrorCode::Timeout,
        format!("{op} timed out (timeout={to:?})"),
    )
}

#[inline(always)]
fn disconnected_err(op: &'static str) -> EngineError {
    EngineError::from_detail(
        ErrorCode::Disconnected,
        format!("{op} failed: channel disconnected"),
    )
}

#[inline(always)]
fn send_err(op: &'static str, e: impl ToString) -> EngineError {
    EngineError::from_detail(
        ErrorCode::Internal,
        format!("{op} send failed: {}", e.to_string()),
    )
}

#[inline(always)]
fn canceled_err(op: &'static str) -> EngineError {
    EngineError::from_detail(
        ErrorCode::Timeout,
        format!("{op} failed: response channel canceled"),
    )
}

#[inline]
fn recv_timeout<T>(
    rx: &Receiver<Result<T, EngineError>>,
    op: &'static str,
) -> Result<T, EngineError> {
    let to = command_timeout();
    match rx.recv_timeout(to) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e),
        Err(RecvTimeoutError::Timeout) => {
            error!("{op} timed out (timeout={to:?})");
            Err(timeout_err(op, to))
        }
        Err(RecvTimeoutError::Disconnected) => {
            error!("{op} failed: channel disconnected");
            Err(disconnected_err(op))
        }
    }
}

#[inline]
async fn oneshot_timeout<T>(
    rx: oneshot::Receiver<Result<T, EngineError>>,
    op: &'static str,
) -> Result<T, EngineError> {
    let to = command_timeout();
    match timeout(to, rx).await {
        Ok(Ok(Ok(v))) => Ok(v),
        Ok(Ok(Err(e))) => Err(e),
        Ok(Err(_canceled)) => Err(canceled_err(op)),
        Err(_elapsed) => Err(timeout_err(op, to)),
    }
}

/// Fire-and-forget GL command.
pub fn send_gl(ctx: &CanvasOpState, cmd: GLCmd) {
    if let Err(e) = ctx.tx.send(RenderCommand::GL(cmd)) {
        error!("send_gl failed: {e}");
    }
}

/// Send a render command with sync response (crossbeam).
pub fn send_render_with_resp_sync<T>(
    ctx: &CanvasOpState,
    op: &'static str,
    build: impl FnOnce(RenderCmdResp<T>) -> RenderCommand,
) -> Result<T, EngineError> {
    let (resp_tx, resp_rx) = bounded(1);

    if let Err(e) = ctx.tx.send(build(RenderCmdResp::Sync(resp_tx))) {
        return Err(send_err(op, e));
    }

    recv_timeout(&resp_rx, op)
}

/// Send a render command with async response (oneshot) + timeout.
pub async fn send_render_with_resp_async<T>(
    ctx: &CanvasOpState,
    op: &'static str,
    build: impl FnOnce(RenderCmdResp<T>) -> RenderCommand,
) -> Result<T, EngineError> {
    let (resp_tx, resp_rx) = oneshot::channel();

    if let Err(e) = ctx.tx.send(build(RenderCmdResp::Async(resp_tx))) {
        return Err(send_err(op, e));
    }

    oneshot_timeout(resp_rx, op).await
}

/// Send a GL command with sync response.
pub fn send_gl_with_resp_sync<T>(
    ctx: &CanvasOpState,
    build: impl FnOnce(RenderCmdResp<T>) -> RenderCommand,
) -> Result<T, EngineError> {
    send_render_with_resp_sync(ctx, OP_GL, build)
}

/// Send a GL command with async response (Promise-style).
pub async fn send_gl_with_resp_async<T>(
    ctx: &CanvasOpState,
    build: impl FnOnce(RenderCmdResp<T>) -> RenderCommand,
) -> Result<T, EngineError> {
    send_render_with_resp_async(ctx, OP_GL, build).await
}

/// Async FS command with oneshot response + timeout.
pub async fn send_fs_with_resp_async<T>(
    tx: &UnboundedSender<IOCmd>,
    build: impl FnOnce(oneshot::Sender<Result<T, EngineError>>) -> IOCmd,
) -> Result<T, EngineError> {
    let (resp_tx, resp_rx) = oneshot::channel();

    if let Err(e) = tx.send(build(resp_tx)) {
        error!("{OP_FS_ASYNC} send failed: {e}");
        return Err(send_err(OP_FS_ASYNC, e));
    }

    let to = command_timeout();
    match timeout(to, resp_rx).await {
        Ok(Ok(Ok(v))) => Ok(v),
        Ok(Ok(Err(e))) => Err(e),
        Ok(Err(_canceled)) => {
            error!("{OP_FS_ASYNC} failed: response channel canceled");
            Err(canceled_err(OP_FS_ASYNC))
        }
        Err(_elapsed) => {
            error!("{OP_FS_ASYNC} timed out (timeout={to:?})");
            Err(timeout_err(OP_FS_ASYNC, to))
        }
    }
}

/// Sync FS command with crossbeam response + timeout.
pub fn send_fs_with_resp_sync<T>(
    tx: &UnboundedSender<IOCmd>,
    build: impl FnOnce(Sender<Result<T, EngineError>>) -> IOCmd,
) -> Result<T, EngineError> {
    let (resp_tx, resp_rx) = bounded(1);

    if let Err(e) = tx.send(build(resp_tx)) {
        error!("{OP_FS_SYNC} send failed: {e}");
        return Err(send_err(OP_FS_SYNC, e));
    }

    recv_timeout(&resp_rx, OP_FS_SYNC)
}
