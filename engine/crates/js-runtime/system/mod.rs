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
    Err(JsErrorBox::generic(
        "openSystemBluetoothSetting:fail not supported",
    ))
}

// ==================== Bluetooth Adapter ====================

#[op2(fast)]
pub fn op_open_bluetooth_adapter(
    state: &mut OpState,
    #[string] options_json: String,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(bt) = services.bluetooth() {
            return bt.open_adapter(&options_json).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic(
        "openBluetoothAdapter:fail not supported",
    ))
}

#[op2(fast)]
pub fn op_close_bluetooth_adapter(state: &mut OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(bt) = services.bluetooth() {
            return bt.close_adapter().map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic(
        "closeBluetoothAdapter:fail not supported",
    ))
}

#[op2]
#[string]
pub fn op_get_bluetooth_adapter_state(state: &mut OpState) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(bt) = services.bluetooth() {
            return bt.get_adapter_state().map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic(
        "getBluetoothAdapterState:fail not supported",
    ))
}

#[op2(fast)]
pub fn op_start_bluetooth_devices_discovery(
    state: &mut OpState,
    #[string] options_json: String,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(bt) = services.bluetooth() {
            return bt
                .start_devices_discovery(&options_json)
                .map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic(
        "startBluetoothDevicesDiscovery:fail not supported",
    ))
}

#[op2(fast)]
pub fn op_stop_bluetooth_devices_discovery(state: &mut OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(bt) = services.bluetooth() {
            return bt.stop_devices_discovery().map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic(
        "stopBluetoothDevicesDiscovery:fail not supported",
    ))
}

#[op2]
#[string]
pub fn op_get_bluetooth_devices(state: &mut OpState) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(bt) = services.bluetooth() {
            return bt.get_devices().map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic(
        "getBluetoothDevices:fail not supported",
    ))
}

#[op2]
#[string]
pub fn op_get_connected_bluetooth_devices(
    state: &mut OpState,
    #[string] options_json: String,
) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(bt) = services.bluetooth() {
            return bt
                .get_connected_devices(&options_json)
                .map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic(
        "getConnectedBluetoothDevices:fail not supported",
    ))
}

#[op2(fast)]
pub fn op_make_bluetooth_pair(
    state: &mut OpState,
    #[string] options_json: String,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(bt) = services.bluetooth() {
            return bt.make_pair(&options_json).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("makeBluetoothPair:fail not supported"))
}

#[op2(fast)]
pub fn op_is_bluetooth_device_paired(
    state: &mut OpState,
    #[string] options_json: String,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(bt) = services.bluetooth() {
            return bt
                .is_device_paired(&options_json)
                .map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic(
        "isBluetoothDevicePaired:fail not supported",
    ))
}

// ==================== Beacon ====================

#[op2(fast)]
pub fn op_start_beacon_discovery(
    state: &mut OpState,
    #[string] options_json: String,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(bt) = services.bluetooth() {
            return bt
                .start_beacon_discovery(&options_json)
                .map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic(
        "startBeaconDiscovery:fail not supported",
    ))
}

#[op2(fast)]
pub fn op_stop_beacon_discovery(state: &mut OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(bt) = services.bluetooth() {
            return bt.stop_beacon_discovery().map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic(
        "stopBeaconDiscovery:fail not supported",
    ))
}

#[op2]
#[string]
pub fn op_get_beacons(state: &mut OpState) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(bt) = services.bluetooth() {
            return bt.get_beacons().map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("getBeacons:fail not supported"))
}

// ==================== App Authorize Setting ====================

#[op2(fast)]
pub fn op_open_app_authorize_setting(state: &mut OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(sys) = services.system_info() {
            return sys
                .open_app_authorize_setting()
                .map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic(
        "openAppAuthorizeSetting:fail not supported",
    ))
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
            return sys
                .get_app_authorization_setting_json()
                .map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic(
        "getAppAuthorizeSetting:fail not supported",
    ))
}

// ==================== Game Log ====================

#[op2(fast)]
pub fn op_game_log_report(
    state: &mut OpState,
    #[string] log_json: String,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(svc) = services.game_log() {
            return svc.report_log(&log_json).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("gameLog.log:fail not supported"))
}

// ==================== Extension Definition ====================

deno_core::extension!(
    host_v8_system,
    deps = [host_v8_base],
    ops = [
        op_open_system_bluetooth_setting,
        op_open_bluetooth_adapter,
        op_close_bluetooth_adapter,
        op_get_bluetooth_adapter_state,
        op_start_bluetooth_devices_discovery,
        op_stop_bluetooth_devices_discovery,
        op_get_bluetooth_devices,
        op_get_connected_bluetooth_devices,
        op_make_bluetooth_pair,
        op_is_bluetooth_device_paired,
        op_start_beacon_discovery,
        op_stop_beacon_discovery,
        op_get_beacons,
        op_open_app_authorize_setting,
        op_get_window_info,
        op_get_system_settings,
        op_get_device_info,
        op_get_app_authorization_setting,
        op_game_log_report,
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
        "09_game_log.js",
        "10_system_info.js",
    ]
);

pub fn system_extensions() -> Vec<Extension> {
    vec![host_v8_system::init()]
}
