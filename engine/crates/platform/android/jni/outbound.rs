use jni::{
    JNIEnv,
    signature::{Primitive, ReturnType},
    sys::jvalue,
};
use shared::{
    device::{Orientation, SystemSettings},
    surface::{SafeArea, WindowInfo},
};

use crate::android::jni::{JAVA_METHOD_CACHE, with_env};

fn call_static_method<R, F>(
    method_name: &str,
    ret: ReturnType,
    handle: F,
    args: &[jvalue],
) -> Result<R, String>
where
    F: FnOnce(&mut JNIEnv, jni::objects::JValueOwned) -> Result<R, String>,
{
    with_env(|env| {
        let cache = JAVA_METHOD_CACHE
            .get()
            .ok_or("NativeExports class cache not initialized")?;

        let method_id = cache
            .get_method_id(method_name)
            .ok_or("Method ID not found")?;

        let class = cache.class();

        let result = unsafe { env.call_static_method_unchecked(class, *method_id, ret, args) };

        match result {
            Ok(val) => handle(env, val),
            Err(e) => {
                if env.exception_check().unwrap_or(false) {
                    env.exception_describe().ok();
                    env.exception_clear().ok();
                }
                Err(format!("Failed to call method '{method_name}': {e}"))
            }
        }
    })
}

pub fn open_bluetooth_settings(host_id: i32) -> Result<(), String> {
    call_static_method(
        "openSystemBluetoothSetting",
        ReturnType::Primitive(Primitive::Void),
        |_env, _| Ok(()),
        &[jvalue { i: host_id }],
    )
}

pub fn open_app_authorize_setting(host_id: i32) -> Result<(), String> {
    call_static_method(
        "openAppAuthorizeSetting",
        ReturnType::Primitive(Primitive::Void),
        |_env, _| Ok(()),
        &[jvalue { i: host_id }],
    )
}

pub fn get_window_info(host_id: i32) -> Result<WindowInfo, String> {
    call_static_method(
        "getWindowInfoBytes",
        ReturnType::Object,
        |env, result| {
            let byte_array = result.l().map_err(|_| "Null byte array from Java")?;
            let bytes = env
                .convert_byte_array(jni::objects::JByteArray::from(byte_array))
                .map_err(|e| format!("Failed to convert byte array: {}", e))?;

            if bytes.len() < 52 {
                return Err("Insufficient data in byte array".to_string());
            }

            let mut window_width =
                i32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32;
            let mut window_height =
                i32::from_ne_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as f32;
            let mut screen_width =
                i32::from_ne_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as f32;
            let mut screen_height =
                i32::from_ne_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as f32;
            let mut status_bar_height =
                i32::from_ne_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) as f32;

            let pixel_ratio_int = i32::from_ne_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
            let pixel_ratio = pixel_ratio_int as f32 / 1000.0;

            let mut screen_top =
                i32::from_ne_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]) as f32;

            let mut safe_area_left =
                i32::from_ne_bytes([bytes[36], bytes[37], bytes[38], bytes[39]]) as f32;
            let mut safe_area_top =
                i32::from_ne_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as f32;
            let mut safe_area_right =
                i32::from_ne_bytes([bytes[44], bytes[45], bytes[46], bytes[47]]) as f32;
            let mut safe_area_bottom =
                i32::from_ne_bytes([bytes[48], bytes[49], bytes[50], bytes[51]]) as f32;

            if pixel_ratio != 0.0 && (pixel_ratio - 1.0).abs() > f32::EPSILON {
                window_width /= pixel_ratio;
                window_height /= pixel_ratio;
                screen_width /= pixel_ratio;
                screen_height /= pixel_ratio;
                status_bar_height /= pixel_ratio;
                screen_top /= pixel_ratio;

                safe_area_left /= pixel_ratio;
                safe_area_top /= pixel_ratio;
                safe_area_right /= pixel_ratio;
                safe_area_bottom /= pixel_ratio;
            }

            Ok(WindowInfo {
                pixel_ratio,
                screen_width,
                screen_height,
                window_width,
                window_height,
                status_bar_height,
                screen_top,
                safe_area: SafeArea {
                    left: safe_area_left,
                    top: safe_area_top,
                    right: safe_area_right,
                    bottom: safe_area_bottom,
                },
            })
        },
        &[jvalue { i: host_id }],
    )
}

pub fn get_system_settings() -> Result<SystemSettings, String> {
    call_static_method(
        "getSystemSettingInfoBytes",
        ReturnType::Object,
        |env, result| {
            let byte_array = result.l().map_err(|_| "Null byte array from Java")?;
            let bytes = env
                .convert_byte_array(jni::objects::JByteArray::from(byte_array))
                .map_err(|e| format!("Failed to convert byte array: {}", e))?;

            if bytes.len() < 4 {
                return Err("Insufficient data in system setting byte array".to_string());
            }

            Ok(SystemSettings {
                bluetooth_enabled: bytes[0] != 0,
                location_enabled: bytes[1] != 0,
                wifi_enabled: bytes[2] != 0,
                orientation: match bytes[3] {
                    1 => Orientation::Portrait,
                    2 => Orientation::Landscape,
                    _ => Orientation::Unknown,
                },
            })
        },
        &[],
    )
}

pub fn get_device_info_json() -> Result<String, String> {
    call_static_method(
        "getDeviceInfoJson",
        ReturnType::Object,
        |env, result| {
            let jstring = result.l().map_err(|_| "Null string from Java")?;
            let json_str = env
                .get_string(&jni::objects::JString::from(jstring))
                .map_err(|e| format!("Failed to convert JSON string: {}", e))?
                .into();
            Ok(json_str)
        },
        &[],
    )
}

pub fn get_app_authorization_setting_json() -> Result<String, String> {
    call_static_method(
        "getAppAuthorizationSettingJson",
        ReturnType::Object,
        |env, result| {
            let jstring = result.l().map_err(|_| "Null string from Java")?;
            let json_str = env
                .get_string(&jni::objects::JString::from(jstring))
                .map_err(|e| format!("Failed to convert authorization JSON string: {}", e))?
                .into();
            Ok(json_str)
        },
        &[],
    )
}
