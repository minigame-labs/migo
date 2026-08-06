use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use crossbeam_channel::{Receiver, RecvTimeoutError, bounded};
use tokio::{sync::oneshot, time::timeout};
use tracing::error;

use crate::{
    error::{EngineError, ErrorCode},
    op_state::CanvasOpState,
};

pub mod ahb;
pub mod audio_cmd;
pub mod camera_frame;
pub mod canvas_id_set;
pub mod color;
pub mod error;
pub mod frame_packet;
pub mod host_cmd;
pub mod io_cmd;
pub mod render_cmd;

pub use self::{
    canvas_id_set::CanvasIdSet,
    frame_packet::{FrameOp, FramePacket, FramePacketBuilder},
    render_cmd::{CanvasBatchPayload, DirtyRect, GlBatchPayload},
};

use self::render_cmd::{GLCmd, RenderCmdResp, RenderCommand};

const OP_GL: &str = "gl command";

/// Default timeout for readback-class sync ops: 10 seconds.
/// Used for `GetImageData`, `ReadPixels`, shader info logs, etc.
/// that can legitimately take a while on slow drivers.
static COMMAND_TIMEOUT_MS: AtomicU64 = AtomicU64::new(10_000);

/// Stricter deadline for latency-sensitive measure-class ops.
///
/// `measureText` and `GetTextLineHeight` are called hundreds of
/// times per frame from UI code auto-sizing labels; a render
/// thread stall should surface as a conservative fallback in JS
/// within a couple of milliseconds rather than eating the full
/// 10 s budget and freezing the whole tick.  Keep it shorter
/// than a 60 Hz frame (16.7 ms) so an overrun is observable as a
/// sub-frame hiccup instead of a visible one.
static MEASURE_TIMEOUT_MS: AtomicU64 = AtomicU64::new(4);

/// Classify a sync op for timeout selection.
///
/// Declared `pub` so downstream crates (e.g. `runtime-v8`)
/// can tag their own op names without threading through
/// `shared::protocol` internals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOpClass {
    /// Measurement / layout — must return fast for interactive
    /// UI; failure degrades to a conservative JS-side estimate.
    Measure,
    /// Readback / creation — can legitimately take longer
    /// (`glReadPixels` a full-screen readback, shader link info).
    Readback,
    /// Legacy / unclassified — uses the readback deadline.
    Default,
}

fn class_for_op(op: &str) -> SyncOpClass {
    // Cheap prefix match — op names are `'static` string
    // constants so the comparison compiles to byte-tests.  Kept
    // here (not on the op site) so we don't have to touch every
    // `send_render_with_resp_*` call to annotate with a class.
    if op.contains("measure_text") || op.contains("get_text_line_height") {
        SyncOpClass::Measure
    } else if op.contains("get_image_data")
        || op.contains("read_pixels")
        || op.contains("get_shader_info_log")
        || op.contains("get_program_info_log")
    {
        SyncOpClass::Readback
    } else {
        SyncOpClass::Default
    }
}

#[inline]
fn timeout_for_op(op: &str) -> Duration {
    match class_for_op(op) {
        SyncOpClass::Measure => Duration::from_millis(MEASURE_TIMEOUT_MS.load(Ordering::Relaxed)),
        SyncOpClass::Readback | SyncOpClass::Default => {
            Duration::from_millis(COMMAND_TIMEOUT_MS.load(Ordering::Relaxed))
        }
    }
}

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

/// Override the measure-class deadline.  Exposed so load-testing
/// code can shorten it to amplify backpressure, or widen it for
/// older devices where the render thread is naturally slower.
pub fn set_measure_timeout(dur: Duration) {
    let ms = dur.as_millis().max(1) as u64;
    MEASURE_TIMEOUT_MS.store(ms, Ordering::Relaxed);
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
    let to = timeout_for_op(op);
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
    let to = timeout_for_op(op);
    match timeout(to, rx).await {
        Ok(Ok(Ok(v))) => Ok(v),
        Ok(Ok(Err(e))) => Err(e),
        Ok(Err(_canceled)) => Err(canceled_err(op)),
        Err(_elapsed) => Err(timeout_err(op, to)),
    }
}

/// Fire-and-forget GL command.  Routed as a draw-class command
/// (`BackpressurePolicy::BlockBounded`) because silently dropping
/// a `bindBuffer` / `uniform*` / `drawElements` produces visible
/// glitches — the next frame's batch is not guaranteed to replay
/// the same mutator sequence.
pub fn send_gl(ctx: &CanvasOpState, cmd: GLCmd) {
    if let Err(e) = ctx.tx.dispatch(RenderCommand::GL(cmd)) {
        error!("send_gl failed: {e}");
    }
}

/// Send a render command with sync response (crossbeam).
///
/// Sync ops MUST be delivered (they're holding a reply channel
/// the caller is blocked on), so we use the bounded-blocking
/// variant that waits up to a hard deadline (~8 ms) before
/// erroring out.  A full-channel error surfaces as a reply
/// timeout in the caller; without this bound a command storm
/// could freeze the host indefinitely.
pub fn send_render_with_resp_sync<T>(
    ctx: &CanvasOpState,
    op: &'static str,
    build: impl FnOnce(RenderCmdResp<T>) -> RenderCommand,
) -> Result<T, EngineError> {
    let (resp_tx, resp_rx) = bounded(1);

    if let Err(e) = ctx
        .tx
        .send_blocking_bounded(build(RenderCmdResp::from_sync(resp_tx)))
    {
        return Err(send_err(op, e));
    }

    let started_at = std::time::Instant::now();
    let result = recv_timeout(&resp_rx, op);
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 5 {
        // Sync GL ops block V8; if the wait crosses 5 ms it is
        // almost certainly a head-of-line stall behind image
        // uploads or a heavy GL command queued earlier in the
        // batch.  Surface it so logcat can correlate with
        // [MigoPerf] readFile spikes.
        tracing::warn!("[MigoPerf][SyncOp] {op} blocked V8 {elapsed_ms}ms");
    }
    result
}

/// Send a render command with async response (oneshot) + timeout.
///
/// Like the sync variant, async sync-ops hold a reply channel and
/// must be delivered.  Use the bounded-blocking sender so a full
/// channel escalates to a timeout instead of an indefinite wait.
pub async fn send_render_with_resp_async<T>(
    ctx: &CanvasOpState,
    op: &'static str,
    build: impl FnOnce(RenderCmdResp<T>) -> RenderCommand,
) -> Result<T, EngineError> {
    let (resp_tx, resp_rx) = oneshot::channel();

    if let Err(e) = ctx
        .tx
        .send_blocking_bounded(build(RenderCmdResp::from_async(resp_tx)))
    {
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
