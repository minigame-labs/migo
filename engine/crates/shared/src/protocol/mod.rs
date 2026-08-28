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

/// Pick a deadline class from an op's name.
///
/// # The mechanism is a substring match, with two consequences worth knowing
///
/// **A rename is a silent 2500x change.** The names are `'static` literals
/// declared next to their ops, so editing `OP_MEASURE_TEXT`'s string — a
/// reasonable-looking tidy-up — moves `measureText` from the 4 ms deadline to
/// the 10 s one, and nothing fails. `every_sync_op_name_lands_in_its_intended_
/// deadline_class` pins the current mapping for exactly that reason.
///
/// **It cannot distinguish the synchronous GL ops, because they share one
/// name.** All 18 `GLCmd` variants that carry a `resp` — `GetParameter`,
/// `CheckFramebufferStatus`, `ClientWaitSync`, `GetQueryParameter`,
/// `GetShaderInfoLog`, `GetProgramInfoLog` and the rest — travel through
/// `send_gl_with_resp_sync`, which passes the single name [`OP_GL`]
/// (`"gl command"`). So they all land on `Default`, and:
///
/// * The `get_shader_info_log` / `get_program_info_log` arms below are
///   unreachable. No op is named either of those; they are waiting for a name
///   that is never passed.
/// * `ClientWaitSync` and `GetQueryParameter` are *designed* to be polled every
///   frame — a fence probe and a timer/occlusion query respectively — and yet
///   they get the 10 s deadline. That is at odds with the policy
///   [`MEASURE_TIMEOUT_MS`] states for per-frame ops: surface a render-thread
///   stall "within a couple of milliseconds rather than eating the full 10 s
///   budget and freezing the whole tick".
///
/// The deadline never fires in normal operation — a healthy round-trip is
/// sub-millisecond, and `send_render_with_resp_sync` already warns past 5 ms.
/// What it bounds is the damage when the render thread is wedged, and for a
/// fence poll that bound is currently 10 s of frozen JS.
///
/// **Not changed here, deliberately.** Shortening it means deciding what a
/// spuriously-failed `clientWaitSync` does to a game's async-readback logic,
/// which is a product call wanting a device measurement, not a constant edit.
/// Making the classification *able* to tell these ops apart is a small change —
/// thread the name through `send_gl_with_resp_sync` — but on its own it alters
/// nothing except log text, so it belongs with the decision rather than before
/// it.
/// `pub` for the same reason [`SyncOpClass`] is: a downstream crate declares
/// the op names, so it is the only place that can check one lands in the class
/// its op needs. Exposing the type without the classifier left that
/// unverifiable — see
/// `context2d::tests::each_sync_op_name_still_selects_the_deadline_its_op_needs`.
pub fn class_for_op(op: &str) -> SyncOpClass {
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

#[cfg(test)]
mod tests {
    use super::{OP_GL, SyncOpClass, class_for_op, timeout_for_op};
    use std::time::Duration;

    /// The two classes must keep mapping to different deadlines, or the
    /// classification stops deciding anything.
    ///
    /// Note what this test deliberately does *not* claim. It cannot check that
    /// any real op's name lands in the right class, because the names are
    /// declared in `runtime-v8` and this crate is upstream of it. A first
    /// version of this test hardcoded the name strings and asserted their
    /// classes — and passed unchanged when `OP_MEASURE_TEXT` was renamed, which
    /// is precisely the failure it was written to catch. The duplicated literals
    /// made divergence invisible rather than loud.
    ///
    /// That check now lives where the constants do:
    /// `context2d::tests::each_sync_op_name_still_selects_the_deadline_its_op_needs`.
    #[test]
    fn the_two_deadline_classes_stay_far_apart() {
        let measure = timeout_for_op("canvas2d measure_text");
        assert_eq!(class_for_op("canvas2d measure_text"), SyncOpClass::Measure);
        assert!(
            measure < Duration::from_millis(17),
            "the measure deadline ({measure:?}) must stay inside a 60 Hz frame, \
             or a stalled measureText becomes a visible hitch rather than a \
             sub-frame one"
        );
        assert!(
            timeout_for_op(OP_GL) > measure * 100,
            "the classes have converged; classifying an op no longer changes \
             its deadline"
        );
    }

    /// The two `Readback` substrings for shader/program info logs are
    /// unreachable, and that is a fact about the code rather than a wish.
    ///
    /// Both ops exist — `GLCmd::GetShaderInfoLog` and `GetProgramInfoLog` carry
    /// a `resp` — but they travel through `send_gl_with_resp_sync`, which names
    /// every synchronous GL op `OP_GL`. Nothing ever passes a string containing
    /// `get_shader_info_log`, so the arm cannot fire.
    ///
    /// Asserted rather than deleted: the arms are harmless, they document an
    /// intent, and they become live the moment someone threads a real name
    /// through the GL sync path. What is worth pinning is that today they do
    /// *not* fire, so nobody reads the classification as evidence that those two
    /// ops are treated separately — they are not.
    #[test]
    fn the_shader_info_log_readback_arms_are_unreachable_through_the_gl_sync_path() {
        // The arms work when given a matching name...
        assert_eq!(
            class_for_op("get_shader_info_log"),
            SyncOpClass::Readback,
            "the arm itself is broken, which is a different bug"
        );
        assert_eq!(class_for_op("get_program_info_log"), SyncOpClass::Readback);

        // ...but the name the GL sync path actually passes is not one of them,
        // so every synchronous GL op — including those two — is `Default`.
        assert_eq!(class_for_op(OP_GL), SyncOpClass::Default);
        assert!(
            !OP_GL.contains("get_shader_info_log") && !OP_GL.contains("get_program_info_log"),
            "OP_GL now names an info-log op; the arms above have become live \
             and this test should be replaced by one covering the new mapping"
        );
    }
}
