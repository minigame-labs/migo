#![allow(non_snake_case)]
#![allow(dead_code)]

/// Escape a string for safe interpolation into a JS single-quoted string literal.
/// Handles all characters that could break out of the string or inject code.
fn escape_for_js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            '`' => out.push_str("\\`"),
            '$' => out.push_str("\\$"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(c),
        }
    }
    out
}

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::Arc;

use jni::objects::{JByteBuffer, JClass, JObject, JString};
use jni::sys::{jdouble, jint, jlong, jobject, jstring};
use jni::{JNIEnv, JavaVM};

use tracing::{error, info};

use core::{send_command_to_host, shutdown_host, spawn_host_thread};
use shared::protocol::host_cmd::{HostCommand, TouchPoint, TouchType};
use shared::surface::SurfaceRef;

use shared::config::InitOptions;

use crate::android::jni::init_jni_env;
use crate::android::logging;
use crate::android::platform::AndroidPlatform;
use crate::android::surface::{
    ANativeWindow_fromSurface, ANativeWindow_getHeight, ANativeWindow_getWidth,
    AndroidSurfaceWrapper,
};

#[unsafe(no_mangle)]
pub extern "system" fn JNI_OnLoad(vm: JavaVM, _reserved: *mut c_void) -> jint {
    // Initialize logging/tracing once.
    logging::init_logging();

    // Initialize ndk-context for cpal/oboe audio backend.
    // This must be done before any audio operations.
    unsafe {
        ndk_context::initialize_android_context(
            vm.get_java_vm_pointer().cast(),
            std::ptr::null_mut(), // activity can be null for audio-only usage
        );
    }

    // Initialize the global JNI environment helper.
    // This also registers Java exports + native exports.
    init_jni_env(vm).expect("Failed to initialize JNI environment");

    jni::sys::JNI_VERSION_1_6
}

pub(crate) extern "system" fn version<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    match env.new_string("0.1.0") {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

pub(crate) extern "system" fn init(
    mut env: JNIEnv,
    _class: JClass,
    surface: jobject,
    options: JObject<'_>,
) -> jint {
    // Convert Java Surface to ANativeWindow*.
    let window = unsafe { ANativeWindow_fromSurface(env.get_native_interface(), surface) };
    if window.is_null() {
        error!("init failed: ANativeWindow_fromSurface returned null");
        return -1;
    }

    let (w, h) = unsafe {
        (
            ANativeWindow_getWidth(window as usize),
            ANativeWindow_getHeight(window as usize),
        )
    };

    // Read required fields from RuntimeConfig
    let cache_dir = match super::get_string_field(&mut env, "cacheDir", &options) {
        Ok(s) => s,
        Err(e) => {
            error!("init failed: read cacheDir error: {}", e);
            return -1;
        }
    };

    let files_dir = match super::get_string_field(&mut env, "filesDir", &options) {
        Ok(s) => s,
        Err(e) => {
            error!("init failed: read filesDir error: {}", e);
            return -1;
        }
    };

    let display_density = match super::get_f32(&mut env, "displayDensity", &options) {
        Ok(v) => v,
        Err(e) => {
            error!("init failed: read displayDensity error: {}", e);
            return -1;
        }
    };

    // Read optional fields with defaults
    let code_cache_dir = super::get_string_field(&mut env, "codeCacheDir", &options)
        .unwrap_or_else(|_| cache_dir.clone());

    let target_fps = super::get_i32(&mut env, "targetFps", &options).unwrap_or(60);

    let debug_enabled = super::get_bool(&mut env, "debugEnabled", &options).unwrap_or(false);

    let log_level_ordinal = super::get_enum_ordinal(
        &mut env,
        "logLevel",
        "Lcom/migo/runtime/RuntimeConfig$LogLevel;",
        &options,
    )
    .unwrap_or(3); // Default to Warn (index 3 in new enum)

    let watchdog_enabled = super::get_bool(&mut env, "watchdogEnabled", &options).unwrap_or(true);
    let watchdog_timeout_secs =
        super::get_i32(&mut env, "watchdogTimeoutSecs", &options).unwrap_or(10);
    let code_signing_enabled =
        super::get_bool(&mut env, "codeSigningEnabled", &options).unwrap_or(true);

    // Read optional code signing public key (hex-encoded Ed25519, 64 chars).
    // Returns None if the field is null, empty, or not present.
    let code_signing_pubkey =
        super::get_optional_string_field(&mut env, "codeSigningPubkey", &options);

    let init_options = InitOptions::new()
        .with_pixel_ratio(display_density)
        .with_cache_dir(PathBuf::from(cache_dir))
        .with_files_dir(PathBuf::from(files_dir))
        .with_code_cache_dir(PathBuf::from(code_cache_dir))
        .with_target_fps(target_fps)
        .with_debug_enabled(debug_enabled)
        .with_log_level(shared::config::LogLevel::from(log_level_ordinal))
        .with_watchdog_enabled(watchdog_enabled)
        .with_watchdog_timeout_secs(watchdog_timeout_secs)
        .with_code_signing_enabled(code_signing_enabled)
        .with_code_signing_pubkey(code_signing_pubkey);

    info!(
        "init: density={}, target_fps={}, debug={}, log_level={:?}",
        display_density,
        target_fps,
        debug_enabled,
        init_options.log_level()
    );

    let platform = Arc::new(AndroidPlatform::new());

    // ANativeWindow_fromSurface returns a new strong ref; wrap as owned (no acquire).
    let android_surface = match unsafe { AndroidSurfaceWrapper::from_surface_owned(window, w, h) } {
        Ok(s) => s,
        Err(e) => {
            error!("init failed: create AndroidSurfaceWrapper error: {}", e);
            return -1;
        }
    };

    let surface_ref: SurfaceRef = Arc::new(android_surface);

    let host_id = spawn_host_thread(surface_ref, platform, init_options);
    match host_id {
        Ok(id) => id,
        Err(e) => {
            error!("Host initialized failed: err={e}");
            -1
        }
    }
}

pub(crate) extern "system" fn updateSurface<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    surface: JObject<'local>,
) {
    // NOTE: ANativeWindow_fromSurface creates a new ANativeWindow reference.
    let raw_surface = surface.into_raw();
    let window = unsafe { ANativeWindow_fromSurface(env.get_native_interface(), raw_surface) };

    if window.is_null() {
        error!("updateSurface failed: ANativeWindow_fromSurface returned null");
        return;
    }

    let (w, h) = unsafe {
        (
            ANativeWindow_getWidth(window as usize),
            ANativeWindow_getHeight(window as usize),
        )
    };

    let android_surface = match unsafe { AndroidSurfaceWrapper::from_surface_owned(window, w, h) } {
        Ok(s) => s,
        Err(e) => {
            error!(
                "updateSurface failed: create AndroidSurfaceWrapper error: {}",
                e
            );
            return;
        }
    };

    let surface_ref: SurfaceRef = Arc::new(android_surface);

    let _ = send_command_to_host(
        host_id,
        HostCommand::UpdateSurface {
            surface: surface_ref,
        },
    );

    info!("Host {} updated surface: {}x{}", host_id, w, h);
}

pub(crate) extern "system" fn onOpenSystemBluetoothSetting<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    enabled: jint,
) {
    let cmd = HostCommand::EvalScript {
        source: format!("_internalOnOpenBluetoothSettingFinished({});", enabled),
    };
    let _ = send_command_to_host(host_id, cmd);
}

pub(crate) extern "system" fn onOpenAppAuthorizeSetting<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    code: jint,
) {
    let cmd = HostCommand::EvalScript {
        source: format!("_internalOnOpenAppAuthorizeSettingFinished({});", code),
    };
    let _ = send_command_to_host(host_id, cmd);
}

pub(crate) extern "system" fn onTouch(
    env: JNIEnv,
    _cls: JClass,
    host_id: jint,
    action: jint,
    time: jlong,
    count: jint,
    buffer: JObject,
) {
    if count <= 0 || count > 10 {
        return;
    }

    let buf = JByteBuffer::from(buffer);

    let addr = match env.get_direct_buffer_address(&buf) {
        Ok(p) => p,
        Err(e) => {
            error!("onTouch failed: get_direct_buffer_address error: {:?}", e);
            return;
        }
    };

    let n = count as usize;
    let expected_size = n * std::mem::size_of::<TouchPoint>();

    // Validate buffer capacity before reading
    let capacity = match env.get_direct_buffer_capacity(&buf) {
        Ok(cap) => cap,
        Err(e) => {
            error!("onTouch failed: get_direct_buffer_capacity error: {:?}", e);
            return;
        }
    };

    if expected_size > capacity {
        error!(
            "onTouch failed: buffer underflow - expected {} bytes, got {} bytes",
            expected_size, capacity
        );
        return;
    }

    // Single memcpy from DirectByteBuffer into fixed inline array — no heap allocation.
    // SAFETY: addr is valid (from get_direct_buffer_address), capacity verified,
    // TouchPoint is repr(C) matching the Java-side packing.
    let mut points = [TouchPoint::default(); 10];
    unsafe {
        std::ptr::copy_nonoverlapping(addr as *const TouchPoint, points.as_mut_ptr(), n);
    }

    let touch_type = match action {
        0 | 5 => TouchType::Start,
        1 | 6 => TouchType::End,
        2 => TouchType::Move,
        3 => TouchType::Cancel,
        _ => TouchType::Move,
    };

    let cmd = HostCommand::OnTouch {
        touch_type,
        count: n as u8,
        points,
        timestamp_ms: time as i64,
    };

    let _ = send_command_to_host(host_id, cmd);
}

pub(crate) extern "system" fn executeScript<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    script: JString<'local>,
) -> jint {
    let script_str = match env.get_string(&script) {
        Ok(s) => s.to_string_lossy().into_owned(),
        Err(e) => {
            error!("executeScript failed: convert JString error: {:?}", e);
            return -1;
        }
    };

    match send_command_to_host(host_id, HostCommand::EvalScript { source: script_str }) {
        Ok(_) => 0,
        Err(e) => {
            error!("executeScript failed: {}", e);
            -1
        }
    }
}

pub(crate) extern "system" fn mod_main<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    game_id: JString<'local>,
    entry: JString<'local>,
) -> jint {
    let game_id = match env.get_string(&game_id) {
        Ok(s) => s.to_string_lossy().into_owned(),
        Err(e) => {
            error!("modMain failed: convert game_id JString error: {:?}", e);
            return -1;
        }
    };

    let entry = match env.get_string(&entry) {
        Ok(s) => s.to_string_lossy().into_owned(),
        Err(e) => {
            error!("modMain failed: convert entry JString error: {:?}", e);
            return -1;
        }
    };

    info!("modMain: game_id={}, entry={}", game_id, entry);

    match send_command_to_host(host_id, HostCommand::EvaluateModule { game_id, entry }) {
        Ok(_) => 0,
        Err(e) => {
            error!("modMain failed: {}", e);
            -1
        }
    }
}

pub(crate) extern "system" fn shutdown<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
) {
    if let Err(e) = shutdown_host(host_id) {
        error!("shutdown failed: host_id={}, error={}", host_id, e);
    } else {
        info!("Host {} shut down successfully", host_id);
    }
}

pub(crate) extern "system" fn onShow(_env: JNIEnv, _class: JClass, host_id: jint) {
    let _ = send_command_to_host(host_id, HostCommand::OnShow);
}

pub(crate) extern "system" fn onHide(_env: JNIEnv, _class: JClass, host_id: jint) {
    let _ = send_command_to_host(host_id, HostCommand::OnHide);
}

pub(crate) extern "system" fn onRestart(_env: JNIEnv, _class: JClass, host_id: jint) {
    let _ = send_command_to_host(host_id, HostCommand::Restart);
}

pub(crate) extern "system" fn onAudioInterruptionBegin(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
) {
    let _ = send_command_to_host(host_id, HostCommand::OnAudioInterruptionBegin);
}

pub(crate) extern "system" fn onAudioInterruptionEnd(_env: JNIEnv, _class: JClass, host_id: jint) {
    let _ = send_command_to_host(host_id, HostCommand::OnAudioInterruptionEnd);
}

pub(crate) extern "system" fn onUserCaptureScreen(_env: JNIEnv, _class: JClass, host_id: jint) {
    let _ = send_command_to_host(host_id, HostCommand::OnUserCaptureScreen);
}

pub(crate) extern "system" fn onModalResult<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    confirm: jint,
    cancel: jint,
) {
    let cmd = HostCommand::EvalScript {
        source: format!("_internalOnModalResult({},{});", confirm, cancel),
    };
    let _ = send_command_to_host(host_id, cmd);
}

pub(crate) extern "system" fn onActionSheetResult<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    tap_index: jint,
) {
    let cmd = HostCommand::EvalScript {
        source: format!("_internalOnActionSheetResult({});", tap_index),
    };
    let _ = send_command_to_host(host_id, cmd);
}

// ==================== Device Sensor ====================

pub(crate) extern "system" fn onDeviceMotionChange(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    alpha: jdouble,
    beta: jdouble,
    gamma: jdouble,
) {
    let _ = send_command_to_host(
        host_id,
        HostCommand::OnDeviceMotionChange {
            alpha: alpha as f64,
            beta: beta as f64,
            gamma: gamma as f64,
        },
    );
}

pub(crate) extern "system" fn onGyroscopeChange(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    x: jdouble,
    y: jdouble,
    z: jdouble,
) {
    let _ = send_command_to_host(
        host_id,
        HostCommand::OnGyroscopeChange {
            x: x as f64,
            y: y as f64,
            z: z as f64,
        },
    );
}

pub(crate) extern "system" fn onDeviceOrientationChange<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    value: JString<'local>,
) {
    let val = env
        .get_string(&value)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let _ = send_command_to_host(
        host_id,
        HostCommand::OnDeviceOrientationChange { value: val },
    );
}

pub(crate) extern "system" fn onCompassChange<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    direction: jdouble,
    accuracy: JString<'local>,
) {
    let acc = env
        .get_string(&accuracy)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string());
    let _ = send_command_to_host(
        host_id,
        HostCommand::OnCompassChange {
            direction: direction as f64,
            accuracy: acc,
        },
    );
}

pub(crate) extern "system" fn onAccelerometerChange(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    x: jdouble,
    y: jdouble,
    z: jdouble,
) {
    let _ = send_command_to_host(
        host_id,
        HostCommand::OnAccelerometerChange {
            x: x as f64,
            y: y as f64,
            z: z as f64,
        },
    );
}

pub(crate) extern "system" fn onNetworkStatusChange<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    is_connected: jni::sys::jboolean,
    network_type: JString<'local>,
) {
    let net_type = env
        .get_string(&network_type)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "none".to_string());
    let _ = send_command_to_host(
        host_id,
        HostCommand::OnNetworkStatusChange {
            is_connected: is_connected != 0,
            network_type: net_type,
        },
    );
}

// ==================== Recorder Events ====================

pub(crate) extern "system" fn onRecorderEvent<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    event_type: JString<'local>,
    json_payload: JString<'local>,
) {
    let evt = env
        .get_string(&event_type)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let payload = env
        .get_string(&json_payload)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "{}".to_string());
    let _ = send_command_to_host(
        host_id,
        HostCommand::RecorderEvent {
            event_type: evt,
            json_payload: payload,
        },
    );
}

pub(crate) extern "system" fn onRecorderFrameData(
    env: JNIEnv,
    _class: JClass,
    host_id: jint,
    frame_data: jni::sys::jbyteArray,
    is_last_frame: jni::sys::jboolean,
) {
    let data =
        match env.convert_byte_array(unsafe { jni::objects::JByteArray::from_raw(frame_data) }) {
            Ok(v) => v,
            Err(e) => {
                error!("onRecorderFrameData: failed to read byte array: {:?}", e);
                return;
            }
        };

    let _ = send_command_to_host(
        host_id,
        HostCommand::RecorderFrameData {
            data,
            is_last_frame: is_last_frame != 0,
        },
    );
}

// ==================== Camera ====================

pub(crate) extern "system" fn onCameraEvent<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    camera_id: jint,
    event_type: JString<'local>,
    json_payload: JString<'local>,
) {
    let evt = env
        .get_string(&event_type)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let payload = env
        .get_string(&json_payload)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "{}".to_string());
    let _ = send_command_to_host(
        host_id,
        HostCommand::CameraEvent {
            camera_id: camera_id as u32,
            event_type: evt,
            json_payload: payload,
        },
    );
}

pub(crate) extern "system" fn onCameraFrameData(
    env: JNIEnv,
    _class: JClass,
    host_id: jint,
    camera_id: jint,
    frame_data: jni::sys::jbyteArray,
    width: jint,
    height: jint,
) {
    let data =
        match env.convert_byte_array(unsafe { jni::objects::JByteArray::from_raw(frame_data) }) {
            Ok(v) => v,
            Err(e) => {
                error!("onCameraFrameData: failed to read byte array: {:?}", e);
                return;
            }
        };

    let _ = send_command_to_host(
        host_id,
        HostCommand::CameraFrameData {
            camera_id: camera_id as u32,
            data,
            width: width as u32,
            height: height as u32,
        },
    );
}

// ==================== Bluetooth Callbacks ====================

pub(crate) extern "system" fn onBluetoothAdapterStateChange(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    available: jni::sys::jboolean,
    discovering: jni::sys::jboolean,
) {
    let _ = send_command_to_host(
        host_id,
        HostCommand::OnBluetoothAdapterStateChange {
            available: available != 0,
            discovering: discovering != 0,
        },
    );
}

pub(crate) extern "system" fn onBluetoothDeviceFound<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    devices_json: JString<'local>,
) {
    let json = env
        .get_string(&devices_json)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "[]".to_string());
    let _ = send_command_to_host(
        host_id,
        HostCommand::OnBluetoothDeviceFound { devices_json: json },
    );
}

pub(crate) extern "system" fn onBeaconUpdate<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    beacons_json: JString<'local>,
) {
    let json = env
        .get_string(&beacons_json)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "[]".to_string());
    let _ = send_command_to_host(host_id, HostCommand::OnBeaconUpdate { beacons_json: json });
}

pub(crate) extern "system" fn onBeaconServiceChange(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    available: jni::sys::jboolean,
    discovering: jni::sys::jboolean,
) {
    let _ = send_command_to_host(
        host_id,
        HostCommand::OnBeaconServiceChange {
            available: available != 0,
            discovering: discovering != 0,
        },
    );
}

// ==================== Keyboard Callbacks ====================

pub(crate) extern "system" fn onKeyboardInput<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    value: JString<'local>,
) {
    let val = env
        .get_string(&value)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let _ = send_command_to_host(host_id, HostCommand::OnKeyboardInput { value: val });
}

pub(crate) extern "system" fn onKeyboardConfirm<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    value: JString<'local>,
) {
    let val = env
        .get_string(&value)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let _ = send_command_to_host(host_id, HostCommand::OnKeyboardConfirm { value: val });
}

pub(crate) extern "system" fn onKeyboardComplete<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    value: JString<'local>,
) {
    let val = env
        .get_string(&value)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let _ = send_command_to_host(host_id, HostCommand::OnKeyboardComplete { value: val });
}

pub(crate) extern "system" fn onKeyboardHeightChange(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    height: jdouble,
) {
    let _ = send_command_to_host(
        host_id,
        HostCommand::OnKeyboardHeightChange {
            height: height as f64,
        },
    );
}

// ==================== Memory Warning Callbacks ====================

pub(crate) extern "system" fn onMemoryWarning(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    level: jint,
) {
    let _ = send_command_to_host(
        host_id,
        HostCommand::OnMemoryWarning {
            level: level as i32,
        },
    );
}

// ==================== Image API Callbacks ====================

pub(crate) extern "system" fn onCompressImageResult<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    result_json: JString<'local>,
) {
    let json = env
        .get_string(&result_json)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| r#"{"error":"failed to read result"}"#.to_string());
    let escaped = json
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    let cmd = HostCommand::EvalScript {
        source: format!("_internalOnCompressImageResult('{}');", escaped),
    };
    let _ = send_command_to_host(host_id, cmd);
}

pub(crate) extern "system" fn onChooseImageResult<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    result_json: JString<'local>,
) {
    let json = env
        .get_string(&result_json)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| r#"{"error":"failed to read result"}"#.to_string());
    let escaped = escape_for_js_string(&json);
    let cmd = HostCommand::EvalScript {
        source: format!("_internalOnChooseImageResult('{}');", escaped),
    };
    let _ = send_command_to_host(host_id, cmd);
}

pub(crate) extern "system" fn onChooseMessageFileResult<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    result_json: JString<'local>,
) {
    let json = env
        .get_string(&result_json)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| r#"{"error":"failed to read result"}"#.to_string());
    let escaped = escape_for_js_string(&json);
    let cmd = HostCommand::EvalScript {
        source: format!("_internalOnChooseMessageFileResult('{}');", escaped),
    };
    let _ = send_command_to_host(host_id, cmd);
}

// ==================== Location Callbacks ====================

pub(crate) extern "system" fn onLocationResult<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    result_json: JString<'local>,
) {
    let json = env
        .get_string(&result_json)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| r#"{"error":"failed to read result"}"#.to_string());
    let escaped = escape_for_js_string(&json);
    let cmd = HostCommand::EvalScript {
        source: format!("_internalOnLocationResult('{}');", escaped),
    };
    let _ = send_command_to_host(host_id, cmd);
}

pub(crate) extern "system" fn onFuzzyLocationResult<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    result_json: JString<'local>,
) {
    let json = env
        .get_string(&result_json)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| r#"{"error":"failed to read result"}"#.to_string());
    let escaped = escape_for_js_string(&json);
    let cmd = HostCommand::EvalScript {
        source: format!("_internalOnFuzzyLocationResult('{}');", escaped),
    };
    let _ = send_command_to_host(host_id, cmd);
}

// ==================== Scan Code Callbacks ====================

pub(crate) extern "system" fn onScanCodeResult<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    result_json: JString<'local>,
) {
    let json = env
        .get_string(&result_json)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| r#"{"error":"failed to read result"}"#.to_string());
    let escaped = json
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    let cmd = HostCommand::EvalScript {
        source: format!("_internalOnScanCodeResult('{}');", escaped),
    };
    let _ = send_command_to_host(host_id, cmd);
}

// ==================== VSync (Choreographer) ====================

pub(crate) extern "system" fn onVsync(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    frame_time_nanos: jlong,
) {
    let ts_ms = frame_time_nanos as f64 / 1_000_000.0;
    core::send_vsync(host_id, ts_ms);
}

// ==================== Debug Stats ====================

pub(crate) extern "system" fn getDebugStats(
    env: JNIEnv,
    _class: JClass,
    host_id: jint,
) -> jni::sys::jbyteArray {
    use std::sync::atomic::Ordering;

    let stats = match shared::stats::get_stats(host_id) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let fps_x10 = stats.fps_x10.load(Ordering::Relaxed);
    let frame_time_us = stats.frame_time_us.load(Ordering::Relaxed);
    let dropped = stats.dropped_frames.load(Ordering::Relaxed);
    let fatal_error = stats.fatal_error_code.load(Ordering::Relaxed);
    let first_frame_ms = stats.first_frame_ms.load(Ordering::Relaxed);

    let mut buf = [0u8; 20];
    buf[0..4].copy_from_slice(&fps_x10.to_le_bytes());
    buf[4..8].copy_from_slice(&frame_time_us.to_le_bytes());
    buf[8..12].copy_from_slice(&dropped.to_le_bytes());
    buf[12..16].copy_from_slice(&fatal_error.to_le_bytes());
    buf[16..20].copy_from_slice(&first_frame_ms.to_le_bytes());

    match env.byte_array_from_slice(&buf) {
        Ok(arr) => arr.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

// ==================== Console Logs ====================

pub(crate) extern "system" fn getConsoleLogs<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    since_cursor: jlong,
) -> jstring {
    let json = shared::console_log::read_console_logs_json(host_id, since_cursor as u64)
        .unwrap_or_else(|| r#"{"logs":[],"cursor":0}"#.to_string());
    match env.new_string(&json) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}
