use deno_core::{extension, op2, Extension, OpState};
use deno_error::JsErrorBox;
use shared::op_state::HostOpState;

use crate::android::jni::*;

#[op2(fast)]
pub fn op_start_device_motion(
    state: &mut OpState,
    #[string] interval: String,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    start_device_motion(host.id, &interval).map_err(|e| JsErrorBox::generic(e))
}

#[op2(fast)]
pub fn op_stop_device_motion(state: &mut OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    stop_device_motion(host.id).map_err(|e| JsErrorBox::generic(e))
}

#[op2(fast)]
pub fn op_start_gyroscope(
    state: &mut OpState,
    #[string] interval: String,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    start_gyroscope(host.id, &interval).map_err(|e| JsErrorBox::generic(e))
}

#[op2(fast)]
pub fn op_stop_gyroscope(state: &mut OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    stop_gyroscope(host.id).map_err(|e| JsErrorBox::generic(e))
}

#[op2]
#[string]
pub fn op_get_battery_info(_: &mut OpState) -> Result<String, JsErrorBox> {
    get_battery_info_json().map_err(|e| JsErrorBox::generic(e))
}

#[op2(fast)]
pub fn op_vibrate_short(_: &mut OpState, #[string] vibrate_type: String) -> Result<i32, JsErrorBox> {
    vibrate_short(&vibrate_type).map_err(|e| JsErrorBox::generic(e))
}

#[op2(fast)]
pub fn op_vibrate_long(_: &mut OpState) -> Result<i32, JsErrorBox> {
    vibrate_long().map_err(|e| JsErrorBox::generic(e))
}

#[op2(fast)]
pub fn op_get_screen_brightness(state: &mut OpState) -> Result<f32, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    get_screen_brightness(host.id).map_err(|e| JsErrorBox::generic(e))
}

#[op2(fast)]
pub fn op_set_screen_brightness(state: &mut OpState, value: f32) -> Result<i32, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    set_screen_brightness(host.id, value).map_err(|e| JsErrorBox::generic(e))
}

#[op2(fast)]
pub fn op_set_keep_screen_on(state: &mut OpState, keep_on: bool) -> Result<i32, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    set_keep_screen_on(host.id, keep_on).map_err(|e| JsErrorBox::generic(e))
}

#[op2(fast)]
pub fn op_set_device_orientation(state: &mut OpState, #[string] value: String) -> Result<i32, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    set_device_orientation(host.id, &value).map_err(|e| JsErrorBox::generic(e))
}

#[op2(fast)]
pub fn op_start_compass(state: &mut OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    start_compass(host.id).map_err(|e| JsErrorBox::generic(e))
}

#[op2(fast)]
pub fn op_stop_compass(state: &mut OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    stop_compass(host.id).map_err(|e| JsErrorBox::generic(e))
}

extension!(host_v8_device_android,
    deps = [host_v8_device],
    ops = [
        op_start_device_motion,
        op_stop_device_motion,
        op_start_gyroscope,
        op_stop_gyroscope,
        op_get_battery_info,
        op_vibrate_short,
        op_vibrate_long,
        op_get_screen_brightness,
        op_set_screen_brightness,
        op_set_keep_screen_on,
        op_set_device_orientation,
        op_start_compass,
        op_stop_compass,
    ],
    esm = [
        dir "android/extensions/base/device",
        "01_device_sensor.js",
        "02_battery.js",
        "03_vibrate.js",
        "04_screen.js",
        "05_compass.js",
    ]
);

pub fn device_extensions() -> Vec<Extension> {
    vec![host_v8_device_android::init_ops_and_esm()]
}
