use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use crossbeam_channel::{RecvTimeoutError, bounded};
use deno_core::{OpState, op2};
use deno_error::JsErrorBox;
use shared::{
    error::{EngineError, ErrorCode},
    op_state::CanvasOpState,
    protocol::render_cmd::{Canvas2DCmd, CanvasCmd, CanvasId, RenderCmdResp, RenderCommand},
    render_command_sender::SendError,
};

const SYNC_TIMEOUT: Duration = Duration::from_millis(1000);

/// Process-global counter for JS-allocated offscreen canvas ids.
///
/// Starts in a high range that the render-thread allocator
/// (`CanvasManager::next_canvas_id`, bumps from 2) cannot reach in
/// any realistic session, so the two pools never collide.  16M is
/// well above any plausible canvas count and well below `u32::MAX`
/// (which is reserved for the snapshot direct path).
const JS_OFFSCREEN_CANVAS_ID_BASE: u32 = 1u32 << 24;
static NEXT_JS_OFFSCREEN_CANVAS_ID: AtomicU32 = AtomicU32::new(JS_OFFSCREEN_CANVAS_ID_BASE);

#[inline]
fn js_err_from_engine(e: EngineError) -> JsErrorBox {
    match &e.detail {
        Some(d) => JsErrorBox::generic(format!("[{:?}] {} ({})", e.code, e.msg, d)),
        None => JsErrorBox::generic(format!("[{:?}] {}", e.code, e.msg)),
    }
}

#[inline]
fn from_crossbeam_recv_err(e: RecvTimeoutError, timeout_msg: &'static str) -> EngineError {
    match e {
        RecvTimeoutError::Timeout => EngineError::new(ErrorCode::Timeout)
            .with_msg(timeout_msg)
            .with_detail("crossbeam recv_timeout".to_string()),
        RecvTimeoutError::Disconnected => EngineError::new(ErrorCode::Disconnected)
            .with_msg("response channel disconnected")
            .with_detail("crossbeam disconnected".to_string()),
    }
}

#[inline]
fn from_send_err(e: SendError) -> EngineError {
    match e {
        SendError::Timeout => EngineError::new(ErrorCode::Timeout)
            .with_msg("send render command timed out")
            .with_detail(e.to_string()),
        SendError::Disconnected => EngineError::new(ErrorCode::Disconnected)
            .with_msg("send render command failed")
            .with_detail(e.to_string()),
        SendError::Overflow => EngineError::new(ErrorCode::Internal)
            .with_msg("send render command overflowed")
            .with_detail(e.to_string()),
    }
}

#[inline]
fn send_canvas_sync<T>(
    ctx: &CanvasOpState,
    build: impl FnOnce(RenderCmdResp<T>) -> RenderCommand,
    timeout_msg: &'static str,
) -> Result<T, JsErrorBox> {
    let (resp_tx, resp_rx) = bounded::<Result<T, EngineError>>(1);
    let resp = RenderCmdResp::from_sync(resp_tx);

    ctx.tx
        .send_blocking_bounded(build(resp))
        .map_err(|e| js_err_from_engine(from_send_err(e)))?;

    match resp_rx.recv_timeout(SYNC_TIMEOUT) {
        Ok(res) => res.map_err(js_err_from_engine),
        Err(e) => Err(js_err_from_engine(from_crossbeam_recv_err(e, timeout_msg))),
    }
}

#[op2(fast)]
pub fn op_create_offscreen_canvas(
    state: &mut OpState,
    #[smi] width: u32,
    #[smi] height: u32,
) -> Result<u32, JsErrorBox> {
    let ctx = state.borrow::<CanvasOpState>();

    // Fire-and-forget: JS allocates the id locally and posts a
    // RegisterOffscreen command without waiting for the render thread.
    // Subsequent ops on this canvas (`getContext`, draw calls) queue
    // on the same FIFO, so ordering is preserved.
    let raw_id = NEXT_JS_OFFSCREEN_CANVAS_ID.fetch_add(1, Ordering::Relaxed);
    let id = CanvasId::from(raw_id);

    let cmd = RenderCommand::Canvas(CanvasCmd::RegisterOffscreen {
        id,
        width,
        height,
    });

    ctx.tx.send(cmd).map_err(|e| {
        js_err_from_engine(
            EngineError::new(ErrorCode::Disconnected)
                .with_msg("send register_offscreen failed")
                .with_detail(e.to_string()),
        )
    })?;

    Ok(raw_id)
}

#[op2]
#[serde]
pub fn op_get_canvas_info(state: &mut OpState, #[smi] id: u32) -> Result<(u32, u32), JsErrorBox> {
    let ctx = state.borrow::<CanvasOpState>();

    send_canvas_sync(
        ctx,
        |resp| RenderCommand::Canvas(CanvasCmd::GetInfo { id, resp }),
        "get_canvas_info timed out",
    )
}

#[op2]
pub fn op_resize_canvas(
    state: &mut OpState,
    #[smi] id: u32,
    w: Option<u32>,
    h: Option<u32>,
) -> Result<(), JsErrorBox> {
    // Route resize through the UnifiedFrameCollector so it interleaves
    // with `FillText` / `TexImage2DFromCanvas2D` in JS-issue order on
    // the render thread.  Previously this went through `ctx.tx` as an
    // immediate `CanvasCmd::ResizeCanvas`, which raced with the
    // collector-buffered draw/upload ops: cocos's text-label pattern
    // (canvas.width=W; fillText; texImage2D) ended up with multiple
    // resizes arriving before the corresponding fillTexts, leaving the
    // pbuffer at the LATEST size while the upload of an EARLIER cycle
    // requested a different size — random-blank labels on arm64.
    //
    // Falls back to the legacy `CanvasCmd::ResizeCanvas` path only when
    // the frame collector is not attached (smoke tests, headless
    // contexts) — production runtimes always have it.
    if let Some(collector) =
        state.try_borrow_mut::<crate::rendering::webgl::frame_collector::UnifiedFrameCollector>()
    {
        collector.push(id, Canvas2DCmd::ResizeCanvas { w, h });
        return Ok(());
    }
    let ctx = state.borrow::<CanvasOpState>();
    let cmd = RenderCommand::Canvas(CanvasCmd::ResizeCanvas { id, w, h });
    ctx.tx.send(cmd).map_err(|e| {
        js_err_from_engine(
            EngineError::new(ErrorCode::Disconnected)
                .with_msg("send resize_canvas failed")
                .with_detail(e.to_string()),
        )
    })?;
    Ok(())
}

#[op2(fast)]
pub fn op_destroy_canvas(state: &mut OpState, #[smi] rid: u32) -> Result<(), JsErrorBox> {
    let ctx = state.borrow::<CanvasOpState>();

    // Onscreen canvas (id=1) is not destroyed by render thread; just drop JS-side ownership.
    if rid == 1 {
        return Ok(());
    }

    let _ = send_canvas_sync(
        ctx,
        |resp| RenderCommand::Canvas(CanvasCmd::DestroyCanvas { id: rid, resp }),
        "destroy_canvas timed out",
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::send_canvas_sync;
    use shared::{
        op_state::CanvasOpState,
        protocol::render_cmd::{CanvasCmd, RenderCmdResp, RenderCommand},
        render_command_sender::CommandSender,
    };

    #[test]
    fn send_canvas_sync_times_out_instead_of_dropping_when_queue_is_full() {
        let (tx, _rx) = CommandSender::new();
        let ctx = CanvasOpState::new(tx);

        for _ in 0..ctx.tx.capacity() {
            ctx.tx
                .send(RenderCommand::Canvas(CanvasCmd::ResizeCanvas {
                    id: 7,
                    w: None,
                    h: None,
                }))
                .unwrap();
        }

        let err = send_canvas_sync(
            &ctx,
            |resp: RenderCmdResp<(u32, u32)>| {
                RenderCommand::Canvas(CanvasCmd::GetInfo { id: 7, resp })
            },
            "get_canvas_info timed out",
        )
        .unwrap_err();

        assert!(
            err.to_string().contains("[Timeout]"),
            "expected timeout error when sync canvas send hits a full queue, got: {err}"
        );
    }
}
