//! Share APIs: showShareMenu, updateShareMenu, onShareAppMessage, shareAppMessage.
//!
//! shareAppMessage delegates to the host via op_share_app_message (Mode C).
//! The host can also trigger share via `_internalTriggerShareAppMessage`.

use deno_core::{Extension, OpState, op2};
use deno_error::JsErrorBox;
use shared::op_state::HostOpState;

#[op2(fast)]
pub fn op_share_app_message(
    state: &mut OpState,
    #[string] options_json: String,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(svc) = services.share() {
            return svc
                .share_app_message(&options_json)
                .map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("shareAppMessage:fail not supported"))
}

deno_core::extension!(
    host_v8_share,
    deps = [host_v8_base],
    ops = [op_share_app_message],
    esm_entry_point = "ext:host_v8_share/99_global_scope.js",
    esm = [
        dir "src/share",
        "01_share.js",
        "99_global_scope.js",
    ],
);

pub fn share_extensions() -> Vec<Extension> {
    vec![host_v8_share::init()]
}
