use deno_core::{extension, op2, Extension, OpState};
use deno_error::JsErrorBox;
use shared::op_state::HostOpState;

use crate::android::jni::*;

#[op2(fast)]
pub fn op_start_network_monitoring(state: &mut OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    start_network_monitoring(host.id).map_err(|e| JsErrorBox::generic(e))
}

#[op2(fast)]
pub fn op_stop_network_monitoring(state: &mut OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    stop_network_monitoring(host.id).map_err(|e| JsErrorBox::generic(e))
}

#[op2]
#[string]
pub fn op_get_network_type(state: &mut OpState) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    get_network_type_json(host.id).map_err(|e| JsErrorBox::generic(e))
}

#[op2]
#[string]
pub fn op_get_local_ip_address(_: &mut OpState) -> Result<String, JsErrorBox> {
    get_local_ip_address_json().map_err(|e| JsErrorBox::generic(e))
}

extension!(host_v8_network_android,
    deps = [host_v8_network],
    ops = [
        op_start_network_monitoring,
        op_stop_network_monitoring,
        op_get_network_type,
        op_get_local_ip_address,
    ],
    esm = [
        dir "android/extensions/base/network",
        "01_network.js",
    ]
);

pub fn network_extensions() -> Vec<Extension> {
    vec![host_v8_network_android::init_ops_and_esm()]
}
