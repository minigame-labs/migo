use deno_core::{Extension, OpState, extension, op2};
use deno_error::JsErrorBox;
use shared::op_state::HostOpState;

use crate::android::jni::*;

#[op2(fast)]
pub fn op_show_toast(state: &mut OpState, #[string] json: String) -> Result<(), JsErrorBox> {
    let options = state.borrow::<HostOpState>();
    show_toast(options.id, &json).map_err(|e| JsErrorBox::generic(e.to_string()))?;
    Ok(())
}

#[op2(fast)]
pub fn op_hide_toast(state: &mut OpState) -> Result<(), JsErrorBox> {
    let options = state.borrow::<HostOpState>();
    hide_toast(options.id).map_err(|e| JsErrorBox::generic(e.to_string()))?;
    Ok(())
}

#[op2(fast)]
pub fn op_show_modal(state: &mut OpState, #[string] json: String) -> Result<(), JsErrorBox> {
    let options = state.borrow::<HostOpState>();
    show_modal(options.id, &json).map_err(|e| JsErrorBox::generic(e.to_string()))?;
    Ok(())
}

#[op2(fast)]
pub fn op_show_loading(state: &mut OpState, #[string] json: String) -> Result<(), JsErrorBox> {
    let options = state.borrow::<HostOpState>();
    show_loading(options.id, &json).map_err(|e| JsErrorBox::generic(e.to_string()))?;
    Ok(())
}

#[op2(fast)]
pub fn op_hide_loading(state: &mut OpState) -> Result<(), JsErrorBox> {
    let options = state.borrow::<HostOpState>();
    hide_loading(options.id).map_err(|e| JsErrorBox::generic(e.to_string()))?;
    Ok(())
}

#[op2(fast)]
pub fn op_show_action_sheet(state: &mut OpState, #[string] json: String) -> Result<(), JsErrorBox> {
    let options = state.borrow::<HostOpState>();
    show_action_sheet(options.id, &json).map_err(|e| JsErrorBox::generic(e.to_string()))?;
    Ok(())
}

extension!(host_v8_ui,
    ops = [op_show_toast, op_hide_toast, op_show_modal, op_show_loading, op_hide_loading, op_show_action_sheet],
    esm = [
        dir "android/extensions/base/ui",
        "01_interaction.js",
    ]
);

pub fn ui_extensions() -> Vec<Extension> {
    vec![host_v8_ui::init()]
}
