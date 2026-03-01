//! System info ops and ESM modules.
//!
//! This module provides cross-platform system information APIs (window info,
//! device info, system settings, bluetooth/authorization settings) using
//! trait-based services injected via `HostOpState.device_services`.

use deno_core::{Extension, OpState, op2};
use deno_error::JsErrorBox;
use shared::op_state::HostOpState;

// ==================== Bluetooth Settings ====================

#[op2(fast)]
pub fn op_open_system_bluetooth_setting(state: &mut OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(sys) = services.system_info() {
            return sys.open_bluetooth_settings().map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("openSystemBluetoothSetting:fail not supported"))
}

// ==================== App Authorize Setting ====================

#[op2(fast)]
pub fn op_open_app_authorize_setting(state: &mut OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(sys) = services.system_info() {
            return sys.open_app_authorize_setting().map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("openAppAuthorizeSetting:fail not supported"))
}

// ==================== Window Info ====================

#[op2]
#[string]
pub fn op_get_window_info(state: &mut OpState) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(sys) = services.system_info() {
            return sys.get_window_info_json().map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("getWindowInfo:fail not supported"))
}

// ==================== System Settings ====================

#[op2]
#[string]
pub fn op_get_system_settings(state: &mut OpState) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(sys) = services.system_info() {
            return sys.get_system_settings_json().map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("getSystemSetting:fail not supported"))
}

// ==================== Device Info ====================

#[op2]
#[string]
pub fn op_get_device_info(state: &mut OpState) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(sys) = services.system_info() {
            return sys.get_device_info_json().map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("getDeviceInfo:fail not supported"))
}

// ==================== App Authorization Setting ====================

#[op2]
#[string]
pub fn op_get_app_authorization_setting(state: &mut OpState) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(sys) = services.system_info() {
            return sys.get_app_authorization_setting_json().map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("getAppAuthorizeSetting:fail not supported"))
}

// ==================== Extension Definition ====================

deno_core::extension!(
    host_v8_system,
    deps = [host_v8_base],
    ops = [
        op_open_system_bluetooth_setting,
        op_open_app_authorize_setting,
        op_get_window_info,
        op_get_system_settings,
        op_get_device_info,
        op_get_app_authorization_setting,
    ],
    esm = [
        dir "system",
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
