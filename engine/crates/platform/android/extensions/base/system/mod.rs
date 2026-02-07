use deno_core::{Extension, OpState, extension, op2};
use deno_error::JsErrorBox;
use shared::{device::SystemSettings, op_state::HostOpState, surface::WindowInfo};

use crate::android::jni::*;

#[op2(fast)]
pub fn op_open_system_bluetooth_setting(state: &mut OpState) -> Result<(), JsErrorBox> {
    let options = state.borrow::<HostOpState>();
    open_bluetooth_settings(options.id).map_err(|e| JsErrorBox::generic(e.to_string()))?;
    Ok(())
}

#[op2(fast)]
pub fn op_open_app_authorize_setting(state: &mut OpState) -> Result<(), JsErrorBox> {
    let options = state.borrow::<HostOpState>();
    open_app_authorize_setting(options.id).map_err(|e| JsErrorBox::generic(e.to_string()))?;
    Ok(())
}

#[op2]
#[serde]
pub fn op_get_window_info(state: &mut OpState) -> Result<WindowInfo, JsErrorBox> {
    let options = state.borrow::<HostOpState>();
    get_window_info(options.id).map_err(|e| JsErrorBox::generic(e.to_string()))
}

#[op2]
#[serde]
pub fn op_get_system_settings(_: &mut OpState) -> Result<SystemSettings, JsErrorBox> {
    get_system_settings().map_err(|e| JsErrorBox::generic(e.to_string()))
}

#[op2]
#[string]
pub fn op_get_device_info(_: &mut OpState) -> Result<String, JsErrorBox> {
    get_device_info_json().map_err(|e| JsErrorBox::generic(e.to_string()))
}

#[op2]
#[string]
pub fn op_get_app_authorization_setting(_: &mut OpState) -> Result<String, JsErrorBox> {
    get_app_authorization_setting_json().map_err(|e| JsErrorBox::generic(e.to_string()))
}

extension!(host_v8_system,
 ops = [op_open_system_bluetooth_setting, op_open_app_authorize_setting, op_get_window_info, op_get_system_settings, op_get_device_info, op_get_app_authorization_setting],
 esm = [
    dir "android/extensions/base/system",
    "01_bluetooth.js",
    "02_authorize.js",
    "03_window_info.js",
    "04_system_settings.js",
    "05_device_info.js",
    "06_benchmark_level.js",
    "07_app_info.js",
    "08_authorize_setting.js",
 ]
);

pub fn system_extensions() -> Vec<Extension> {
    vec![host_v8_system::init()]
}
