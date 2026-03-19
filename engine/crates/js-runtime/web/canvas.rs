use std::time::Duration;

use crossbeam_channel::{RecvTimeoutError, bounded};
use deno_core::{OpState, op2};
use deno_error::JsErrorBox;
use shared::{
    error::{EngineError, ErrorCode},
    op_state::CanvasOpState,
    protocol::render_cmd::{CanvasCmd, RenderCmdResp, RenderCommand},
};

const SYNC_TIMEOUT: Duration = Duration::from_millis(1000);

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
fn send_canvas_sync<T>(
    ctx: &CanvasOpState,
    build: impl FnOnce(RenderCmdResp<T>) -> RenderCommand,
    timeout_msg: &'static str,
) -> Result<T, JsErrorBox> {
    let (resp_tx, resp_rx) = bounded::<Result<T, EngineError>>(1);
    let resp = RenderCmdResp::Sync(resp_tx);

    ctx.tx.send(build(resp)).map_err(|e| {
        js_err_from_engine(
            EngineError::new(ErrorCode::Disconnected)
                .with_msg("send render command failed")
                .with_detail(e.to_string()),
        )
    })?;

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

    send_canvas_sync(
        ctx,
        |resp| {
            RenderCommand::Canvas(CanvasCmd::CreateOffscreen {
                width,
                height,
                resp,
            })
        },
        "create_offscreen_canvas timed out",
    )
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
