#![allow(non_snake_case)]

use shared::js_escape::build_eval_script;

use std::borrow::Cow;
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::Arc;

use jni::objects::{JByteBuffer, JClass, JObject, JString};

/// Parse the internal subPackagesJson field into a Vec of (name, root) pairs.
fn parse_sub_packages(json: Option<String>) -> Vec<(String, String)> {
    let json = match json {
        Some(s) if !s.is_empty() => s,
        _ => return Vec::new(),
    };
    #[derive(serde::Deserialize)]
    struct Entry {
        name: String,
        root: String,
    }
    deno_core::serde_json::from_str::<Vec<Entry>>(&json)
        .map(|v| v.into_iter().map(|e| (e.name, e.root)).collect())
        .unwrap_or_default()
}

/// Convert a JNI string to `Cow<'static, str>`.
/// Returns a borrowed `&'static str` for known constant values,
/// avoiding heap allocation in the common case.
fn jni_string_to_cow(
    env: &mut JNIEnv,
    jstr: &JString,
    known: &[&'static str],
) -> Cow<'static, str> {
    let s: String = env
        .get_string(jstr)
        .map(|s| s.into())
        .unwrap_or_default();
    for &k in known {
        if s == k {
            return Cow::Borrowed(k);
        }
    }
    Cow::Owned(s)
}

// ---------------------------------------------------------------------------
// Helper: forward a JSON result string from JNI to JS via EvalScript.
// Used by all "Mode C" callbacks that receive a JSON result from Java and
// need to invoke a global `_internalOn*('escaped_json')` function in V8.
// ---------------------------------------------------------------------------

fn forward_json_result_to_js(
    env: &mut JNIEnv,
    host_id: jint,
    result_json: &JString,
    js_callback: &str,
    fallback_json: &str,
) {
    let json: String = env
        .get_string(result_json)
        .map(|s| s.into())
        .unwrap_or_else(|_| fallback_json.to_string());
    let cmd = HostCommand::EvalScript {
        source: build_eval_script(js_callback, &json),
    };
    let _ = send_command_to_host(host_id, cmd);
}

/// Generate a JNI `extern "system"` callback that forwards a JSON string
/// result to JS via `forward_json_result_to_js`.
macro_rules! jni_json_callback {
    ($fn_name:ident, $js_callback:literal) => {
        jni_json_callback!(
            $fn_name,
            $js_callback,
            r#"{"error":"failed to read result"}"#
        );
    };
    ($fn_name:ident, $js_callback:literal, $fallback:expr) => {
        pub(crate) extern "system" fn $fn_name<'local>(
            mut env: JNIEnv<'local>,
            _class: JClass<'local>,
            host_id: jint,
            result_json: JString<'local>,
        ) {
            jni_safe!(stringify!($fn_name), {
                forward_json_result_to_js(&mut env, host_id, &result_json, $js_callback, $fallback);
            });
        }
    };
}
use jni::sys::{jdouble, jint, jlong, jobject, jstring};
use jni::{JNIEnv, JavaVM};

use tracing::{error, info};

use core::{send_command_to_host, shutdown_host, spawn_host_thread};
use shared::protocol::host_cmd::{
    BleCharacteristicData, HostCommand, TouchData, TouchPoint, TouchType,
};
use shared::surface::SurfaceRef;

use shared::config::InitOptions;

use crate::android::jni::init_jni_env;
use crate::android::jni::jni_safe;
use crate::android::logging;
use crate::android::platform::AndroidPlatform;
use crate::android::surface::{
    ANativeWindow_fromSurface, ANativeWindow_getHeight, ANativeWindow_getWidth,
    ANativeWindow_setBuffersGeometry, AndroidSurfaceWrapper,
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
    jni_safe!("version", std::ptr::null_mut(), {
        match env.new_string("0.1.0") {
            Ok(s) => s.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    })
}

pub(crate) extern "system" fn init(
    mut env: JNIEnv,
    _class: JClass,
    surface: jobject,
    options: JObject<'_>,
) -> jint {
    jni_safe!("init", -1, {
        // Convert Java Surface to ANativeWindow*.
        let window = unsafe { ANativeWindow_fromSurface(env.get_native_interface(), surface) };
        if window.is_null() {
            error!("init failed: ANativeWindow_fromSurface returned null");
            return -1;
        }

        let (raw_w, raw_h) = unsafe {
            (
                ANativeWindow_getWidth(window),
                ANativeWindow_getHeight(window),
            )
        };
        if raw_w <= 0 || raw_h <= 0 {
            error!(
                "init failed: ANativeWindow_getWidth/Height returned invalid size: {}x{}",
                raw_w, raw_h
            );
            return -1;
        }
        let (w, h) = (raw_w as u32, raw_h as u32);

        // Normalize native window buffer geometry to the observed dimensions.
        // This helps avoid stale rotated geometry during startup transitions.
        let set_geo_rc = unsafe { ANativeWindow_setBuffersGeometry(window, raw_w, raw_h, 0) };
        if set_geo_rc != 0 {
            tracing::warn!(
                "init: ANativeWindow_setBuffersGeometry({}x{}) failed: {}",
                raw_w,
                raw_h,
                set_geo_rc
            );
        }

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

        let watchdog_enabled =
            super::get_bool(&mut env, "watchdogEnabled", &options).unwrap_or(true);
        let watchdog_timeout_secs =
            super::get_i32(&mut env, "watchdogTimeoutSecs", &options).unwrap_or(10);
        let code_signing_enabled =
            super::get_bool(&mut env, "codeSigningEnabled", &options).unwrap_or(true);

        // Read optional code signing public key (hex-encoded Ed25519, 64 chars).
        // Returns None if the field is null, empty, or not present.
        let code_signing_pubkey =
            super::get_optional_string_field(&mut env, "codeSigningPubkey", &options);

        // Read optional game config fields
        let sub_packages = parse_sub_packages(super::get_optional_string_field(
            &mut env,
            "subPackagesJson",
            &options,
        ));
        let workers_path = super::get_optional_string_field(&mut env, "workersPath", &options);

        let log_level = shared::config::LogLevel::from(log_level_ordinal);

        let init_options = InitOptions::new()
            .with_pixel_ratio(display_density)
            .with_cache_dir(PathBuf::from(cache_dir))
            .with_files_dir(PathBuf::from(files_dir))
            .with_code_cache_dir(PathBuf::from(code_cache_dir))
            .with_target_fps(target_fps)
            .with_debug_enabled(debug_enabled)
            .with_log_level(log_level)
            .with_watchdog_enabled(watchdog_enabled)
            .with_watchdog_timeout_secs(watchdog_timeout_secs)
            .with_code_signing_enabled(code_signing_enabled)
            .with_code_signing_pubkey(code_signing_pubkey)
            .with_sub_packages(sub_packages)
            .with_workers_path(workers_path);

        // Apply RuntimeConfig log level to the tracing subscriber.
        logging::update_log_level(log_level);

        info!(
            "init: density={}, target_fps={}, debug={}, log_level={:?}",
            display_density,
            target_fps,
            debug_enabled,
            init_options.log_level()
        );

        let platform = Arc::new(AndroidPlatform::new());

        // ANativeWindow_fromSurface returns a new strong ref; wrap as owned (no acquire).
        let android_surface =
            match unsafe { AndroidSurfaceWrapper::from_surface_owned(window, w, h) } {
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
    })
}

pub(crate) extern "system" fn updateSurface<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    surface: JObject<'local>,
    width: jint,
    height: jint,
) {
    jni_safe!("updateSurface", {
        // NOTE: ANativeWindow_fromSurface creates a new ANativeWindow reference.
        let raw_surface = surface.into_raw();
        let window = unsafe { ANativeWindow_fromSurface(env.get_native_interface(), raw_surface) };

        if window.is_null() {
            error!("updateSurface failed: ANativeWindow_fromSurface returned null");
            return;
        }

        let (raw_w, raw_h) = unsafe {
            (
                ANativeWindow_getWidth(window),
                ANativeWindow_getHeight(window),
            )
        };
        if raw_w <= 0 || raw_h <= 0 {
            error!(
                "updateSurface failed: ANativeWindow_getWidth/Height returned invalid size: {}x{}",
                raw_w, raw_h
            );
            return;
        }

        let mut w = raw_w as u32;
        let mut h = raw_h as u32;
        if width > 0 && height > 0 {
            let provided_w = width as u32;
            let provided_h = height as u32;
            if provided_w != w || provided_h != h {
                tracing::warn!(
                    "updateSurface size mismatch: provided={}x{}, native={}x{}; using provided size",
                    provided_w,
                    provided_h,
                    w,
                    h
                );
            }
            w = provided_w;
            h = provided_h;
        }

        let set_geo_rc =
            unsafe { ANativeWindow_setBuffersGeometry(window, w as i32, h as i32, 0) };
        if set_geo_rc != 0 {
            tracing::warn!(
                "updateSurface: ANativeWindow_setBuffersGeometry({}x{}) failed: {}",
                w,
                h,
                set_geo_rc
            );
        }

        let android_surface =
            match unsafe { AndroidSurfaceWrapper::from_surface_owned(window, w, h) } {
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

        if let Err(e) = send_command_to_host(
            host_id,
            HostCommand::UpdateSurface {
                surface: surface_ref,
            },
        ) {
            error!("Failed to send UpdateSurface for host {host_id}: {e}");
        }

        info!("Host {} updated surface: {}x{}", host_id, w, h);
    });
}

pub(crate) extern "system" fn onSurfaceDestroyed(_env: JNIEnv, _class: JClass, host_id: jint) {
    jni_safe!("onSurfaceDestroyed", {
        if let Err(e) = send_command_to_host(host_id, HostCommand::SurfaceDestroyed) {
            error!("Failed to send SurfaceDestroyed for host {host_id}: {e}");
        }
    });
}

pub(crate) extern "system" fn onOpenSystemBluetoothSetting<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    enabled: jint,
) {
    jni_safe!("onOpenSystemBluetoothSetting", {
        let json = if enabled >= 0 {
            format!(
                r#"{{"errMsg":"openBluetoothAdapterSetting:ok","code":{}}}"#,
                enabled
            )
        } else {
            format!(
                r#"{{"errMsg":"openBluetoothAdapterSetting:fail","code":{}}}"#,
                enabled
            )
        };
        let escaped = json
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n")
            .replace('\r', "\\r");
        let cmd = HostCommand::EvalScript {
            source: format!("_internalOnOpenBluetoothSettingResult('{}');", escaped),
        };
        let _ = send_command_to_host(host_id, cmd);
    });
}

pub(crate) extern "system" fn onOpenAppAuthorizeSetting<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    code: jint,
) {
    jni_safe!("onOpenAppAuthorizeSetting", {
        let cmd = HostCommand::EvalScript {
            source: format!("_internalOnOpenAppAuthorizeSettingFinished({});", code),
        };
        let _ = send_command_to_host(host_id, cmd);
    });
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
    jni_safe!("onTouch", {
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

        let cmd = HostCommand::OnTouch(Box::new(TouchData {
            touch_type,
            count: n as u8,
            points,
            timestamp_ms: time as i64,
        }));

        let _ = send_command_to_host(host_id, cmd);
    });
}

pub(crate) extern "system" fn executeScript<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    script: JString<'local>,
) -> jint {
    jni_safe!("executeScript", -1, {
        let script_str: String = match env.get_string(&script) {
            Ok(s) => s.into(),
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
    })
}

pub(crate) extern "system" fn mod_main<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    game_id: JString<'local>,
    entry: JString<'local>,
) -> jint {
    jni_safe!("mod_main", -1, {
        let game_id: String = match env.get_string(&game_id) {
            Ok(s) => s.into(),
            Err(e) => {
                error!("modMain failed: convert game_id JString error: {:?}", e);
                return -1;
            }
        };

        let entry: String = match env.get_string(&entry) {
            Ok(s) => s.into(),
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
    })
}

pub(crate) extern "system" fn shutdown<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
) {
    jni_safe!("shutdown", {
        if let Err(e) = shutdown_host(host_id) {
            error!("shutdown failed: host_id={}, error={}", host_id, e);
        } else {
            info!("Host {} shut down successfully", host_id);
        }
    });
}

pub(crate) extern "system" fn onShow<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    options_json: JString<'local>,
) {
    jni_safe!("onShow", {
        let options_json = if options_json.is_null() {
            None
        } else {
            env.get_string(&options_json)
                .ok()
                .map(|s| s.into())
        };

        if let Some(json) = options_json.as_ref() {
            info!(
                "Host {} onShow received (options_json_bytes={})",
                host_id,
                json.len()
            );
        } else {
            info!("Host {} onShow received (options_json=<none>)", host_id);
        }

        if let Err(e) = send_command_to_host(host_id, HostCommand::OnShow { options_json }) {
            error!("Failed to send OnShow for host {host_id}: {e}");
        }
    });
}

pub(crate) extern "system" fn onHide(_env: JNIEnv, _class: JClass, host_id: jint) {
    jni_safe!("onHide", {
        info!("Host {} onHide received", host_id);
        if let Err(e) = send_command_to_host(host_id, HostCommand::OnHide) {
            error!("Failed to send OnHide for host {host_id}: {e}");
        }
    });
}

pub(crate) extern "system" fn onRestart(_env: JNIEnv, _class: JClass, host_id: jint) {
    jni_safe!("onRestart", {
        if let Err(e) = send_command_to_host(host_id, HostCommand::Restart) {
            error!("Failed to send Restart for host {host_id}: {e}");
        }
    });
}

pub(crate) extern "system" fn onAudioInterruptionBegin(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
) {
    jni_safe!("onAudioInterruptionBegin", {
        let _ = send_command_to_host(host_id, HostCommand::OnAudioInterruptionBegin);
    });
}

pub(crate) extern "system" fn onAudioInterruptionEnd(_env: JNIEnv, _class: JClass, host_id: jint) {
    jni_safe!("onAudioInterruptionEnd", {
        let _ = send_command_to_host(host_id, HostCommand::OnAudioInterruptionEnd);
    });
}

pub(crate) extern "system" fn onUserCaptureScreen(_env: JNIEnv, _class: JClass, host_id: jint) {
    jni_safe!("onUserCaptureScreen", {
        let _ = send_command_to_host(host_id, HostCommand::OnUserCaptureScreen);
    });
}

pub(crate) extern "system" fn onModalResult<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    confirm: jint,
    cancel: jint,
) {
    jni_safe!("onModalResult", {
        let cmd = HostCommand::EvalScript {
            source: format!("_internalOnModalResult({},{});", confirm, cancel),
        };
        let _ = send_command_to_host(host_id, cmd);
    });
}

pub(crate) extern "system" fn onActionSheetResult<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    tap_index: jint,
) {
    jni_safe!("onActionSheetResult", {
        let cmd = HostCommand::EvalScript {
            source: format!("_internalOnActionSheetResult({});", tap_index),
        };
        let _ = send_command_to_host(host_id, cmd);
    });
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
    jni_safe!("onDeviceMotionChange", {
        let _ = send_command_to_host(
            host_id,
            HostCommand::OnDeviceMotionChange {
                alpha: alpha as f64,
                beta: beta as f64,
                gamma: gamma as f64,
            },
        );
    });
}

pub(crate) extern "system" fn onGyroscopeChange(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    x: jdouble,
    y: jdouble,
    z: jdouble,
) {
    jni_safe!("onGyroscopeChange", {
        let _ = send_command_to_host(
            host_id,
            HostCommand::OnGyroscopeChange {
                x: x as f64,
                y: y as f64,
                z: z as f64,
            },
        );
    });
}

pub(crate) extern "system" fn onDeviceOrientationChange<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    value: JString<'local>,
) {
    jni_safe!("onDeviceOrientationChange", {
        static KNOWN: &[&str] = &["portrait", "landscape", "landscapeReverse"];
        let val = jni_string_to_cow(&mut env, &value, KNOWN);
        let _ = send_command_to_host(
            host_id,
            HostCommand::OnDeviceOrientationChange { value: val },
        );
    });
}

pub(crate) extern "system" fn onCompassChange<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    direction: jdouble,
    accuracy: JString<'local>,
) {
    jni_safe!("onCompassChange", {
        static KNOWN: &[&str] = &["high", "medium", "low", "no-contact", "unreliable"];
        let acc = jni_string_to_cow(&mut env, &accuracy, KNOWN);
        let _ = send_command_to_host(
            host_id,
            HostCommand::OnCompassChange {
                direction: direction as f64,
                accuracy: acc,
            },
        );
    });
}

pub(crate) extern "system" fn onAccelerometerChange(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    x: jdouble,
    y: jdouble,
    z: jdouble,
) {
    jni_safe!("onAccelerometerChange", {
        let _ = send_command_to_host(
            host_id,
            HostCommand::OnAccelerometerChange {
                x: x as f64,
                y: y as f64,
                z: z as f64,
            },
        );
    });
}

pub(crate) extern "system" fn onNetworkStatusChange<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    is_connected: jni::sys::jboolean,
    network_type: JString<'local>,
) {
    jni_safe!("onNetworkStatusChange", {
        static KNOWN: &[&str] = &["wifi", "2g", "3g", "4g", "5g", "unknown", "none"];
        let net = jni_string_to_cow(&mut env, &network_type, KNOWN);
        let _ = send_command_to_host(
            host_id,
            HostCommand::OnNetworkStatusChange {
                is_connected: is_connected != 0,
                network_type: net,
            },
        );
    });
}

// ==================== Recorder Events ====================

pub(crate) extern "system" fn onRecorderEvent<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    event_type: JString<'local>,
    json_payload: JString<'local>,
) {
    jni_safe!("onRecorderEvent", {
        let evt: String = env
            .get_string(&event_type)
            .map(|s| s.into())
            .unwrap_or_default();
        let payload: String = env
            .get_string(&json_payload)
            .map(|s| s.into())
            .unwrap_or_else(|_| "{}".to_string());
        let _ = send_command_to_host(
            host_id,
            HostCommand::RecorderEvent {
                event_type: evt,
                json_payload: payload,
            },
        );
    });
}

pub(crate) extern "system" fn onRecorderFrameData(
    env: JNIEnv,
    _class: JClass,
    host_id: jint,
    frame_data: jni::sys::jbyteArray,
    is_last_frame: jni::sys::jboolean,
) {
    jni_safe!("onRecorderFrameData", {
        let data = match env
            .convert_byte_array(unsafe { jni::objects::JByteArray::from_raw(frame_data) })
        {
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
    });
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
    jni_safe!("onCameraEvent", {
        let evt: String = env
            .get_string(&event_type)
            .map(|s| s.into())
            .unwrap_or_default();
        let payload: String = env
            .get_string(&json_payload)
            .map(|s| s.into())
            .unwrap_or_else(|_| "{}".to_string());
        let _ = send_command_to_host(
            host_id,
            HostCommand::CameraEvent {
                camera_id: camera_id as u32,
                event_type: evt,
                json_payload: payload,
            },
        );
    });
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
    jni_safe!("onCameraFrameData", {
        let data = match env
            .convert_byte_array(unsafe { jni::objects::JByteArray::from_raw(frame_data) })
        {
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
    });
}

// ==================== Bluetooth Callbacks ====================

pub(crate) extern "system" fn onBluetoothAdapterStateChange(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    available: jni::sys::jboolean,
    discovering: jni::sys::jboolean,
) {
    jni_safe!("onBluetoothAdapterStateChange", {
        let _ = send_command_to_host(
            host_id,
            HostCommand::OnBluetoothAdapterStateChange {
                available: available != 0,
                discovering: discovering != 0,
            },
        );
    });
}

pub(crate) extern "system" fn onBluetoothDeviceFound<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    devices_json: JString<'local>,
) {
    jni_safe!("onBluetoothDeviceFound", {
        let json: String = env
            .get_string(&devices_json)
            .map(|s| s.into())
            .unwrap_or_else(|_| "[]".to_string());
        let _ = send_command_to_host(
            host_id,
            HostCommand::OnBluetoothDeviceFound { devices_json: json },
        );
    });
}

pub(crate) extern "system" fn onBeaconUpdate<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    beacons_json: JString<'local>,
) {
    jni_safe!("onBeaconUpdate", {
        let json: String = env
            .get_string(&beacons_json)
            .map(|s| s.into())
            .unwrap_or_else(|_| "[]".to_string());
        let _ =
            send_command_to_host(host_id, HostCommand::OnBeaconUpdate { beacons_json: json });
    });
}

pub(crate) extern "system" fn onBeaconServiceChange(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    available: jni::sys::jboolean,
    discovering: jni::sys::jboolean,
) {
    jni_safe!("onBeaconServiceChange", {
        let _ = send_command_to_host(
            host_id,
            HostCommand::OnBeaconServiceChange {
                available: available != 0,
                discovering: discovering != 0,
            },
        );
    });
}

// ==================== BLE GATT Callbacks ====================

pub(crate) extern "system" fn onBLEConnectionStateChange<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    device_id: JString<'local>,
    connected: jni::sys::jboolean,
) {
    jni_safe!("onBLEConnectionStateChange", {
        let dev: String = env
            .get_string(&device_id)
            .map(|s| s.into())
            .unwrap_or_default();
        let _ = send_command_to_host(
            host_id,
            HostCommand::OnBLEConnectionStateChange {
                device_id: dev,
                connected: connected != 0,
            },
        );
    });
}

pub(crate) extern "system" fn onBLECharacteristicValueChange<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    device_id: JString<'local>,
    service_id: JString<'local>,
    characteristic_id: JString<'local>,
    value: jni::objects::JByteArray<'local>,
) {
    jni_safe!("onBLECharacteristicValueChange", {
        let dev: String = env
            .get_string(&device_id)
            .map(|s| s.into())
            .unwrap_or_default();
        let svc: String = env
            .get_string(&service_id)
            .map(|s| s.into())
            .unwrap_or_default();
        let chr: String = env
            .get_string(&characteristic_id)
            .map(|s| s.into())
            .unwrap_or_default();
        let val: Vec<u8> = env.convert_byte_array(&value).unwrap_or_default();
        let _ = send_command_to_host(
            host_id,
            HostCommand::OnBLECharacteristicValueChange(Box::new(BleCharacteristicData {
                device_id: dev,
                service_id: svc,
                characteristic_id: chr,
                value: val,
            })),
        );
    });
}

pub(crate) extern "system" fn onBLEMTUChange<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    device_id: JString<'local>,
    mtu: jint,
) {
    jni_safe!("onBLEMTUChange", {
        let dev: String = env
            .get_string(&device_id)
            .map(|s| s.into())
            .unwrap_or_default();
        let _ = send_command_to_host(
            host_id,
            HostCommand::OnBLEMTUChange {
                device_id: dev,
                mtu: mtu as u32,
            },
        );
    });
}

// ==================== Keyboard Callbacks ====================

pub(crate) extern "system" fn onKeyboardInput<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    value: JString<'local>,
) {
    jni_safe!("onKeyboardInput", {
        let val: String = env
            .get_string(&value)
            .map(|s| s.into())
            .unwrap_or_default();
        let _ = send_command_to_host(host_id, HostCommand::OnKeyboardInput { value: val });
    });
}

pub(crate) extern "system" fn onKeyboardConfirm<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    value: JString<'local>,
) {
    jni_safe!("onKeyboardConfirm", {
        let val: String = env
            .get_string(&value)
            .map(|s| s.into())
            .unwrap_or_default();
        let _ = send_command_to_host(host_id, HostCommand::OnKeyboardConfirm { value: val });
    });
}

pub(crate) extern "system" fn onKeyboardComplete<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    value: JString<'local>,
) {
    jni_safe!("onKeyboardComplete", {
        let val: String = env
            .get_string(&value)
            .map(|s| s.into())
            .unwrap_or_default();
        let _ = send_command_to_host(host_id, HostCommand::OnKeyboardComplete { value: val });
    });
}

pub(crate) extern "system" fn onKeyboardHeightChange(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    height: jdouble,
) {
    jni_safe!("onKeyboardHeightChange", {
        let _ = send_command_to_host(
            host_id,
            HostCommand::OnKeyboardHeightChange {
                height: height as f64,
            },
        );
    });
}

// ==================== Memory Warning Callbacks ====================

pub(crate) extern "system" fn onMemoryWarning(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    level: jint,
) {
    jni_safe!("onMemoryWarning", {
        let _ = send_command_to_host(
            host_id,
            HostCommand::OnMemoryWarning {
                level: level as i32,
            },
        );
    });
}

// ==================== ADPF Thermal Callbacks ====================

pub(crate) extern "system" fn onThermalStatusChanged(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    status: jint,
) {
    jni_safe!("onThermalStatusChanged", {
        let _ = send_command_to_host(
            host_id,
            HostCommand::OnThermalStatusChanged { status: status as i32 },
        );
    });
}

// ==================== Image API Callbacks ====================

jni_json_callback!(onCompressImageResult, "_internalOnCompressImageResult");
jni_json_callback!(onChooseImageResult, "_internalOnChooseImageResult");
jni_json_callback!(
    onChooseMessageFileResult,
    "_internalOnChooseMessageFileResult"
);

// ==================== Location Callbacks ====================

jni_json_callback!(onLocationResult, "_internalOnLocationResult");
jni_json_callback!(onFuzzyLocationResult, "_internalOnFuzzyLocationResult");

// ==================== Scan Code Callbacks ====================

jni_json_callback!(onScanCodeResult, "_internalOnScanCodeResult");

// ==================== Auth Callbacks ====================

jni_json_callback!(onLoginResult, "_internalOnLoginResult");
jni_json_callback!(onCheckSessionResult, "_internalOnCheckSessionResult");
jni_json_callback!(onGetUserInfoResult, "_internalOnGetUserInfoResult");
jni_json_callback!(onGetPhoneNumberResult, "_internalOnGetPhoneNumberResult");

// ==================== Subpackage Callbacks ====================

jni_json_callback!(
    onSubpackageProgress,
    "_internalOnSubpackageProgress",
    r#"{"requestId":0}"#
);
jni_json_callback!(
    onSubpackageResult,
    "_internalOnSubpackageResult",
    r#"{"requestId":0,"error":"failed to read result"}"#
);

// ==================== VSync (Choreographer) ====================

pub(crate) extern "system" fn onVsync(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    frame_time_nanos: jlong,
) {
    jni_safe!("onVsync", {
        let frame_time_ms = frame_time_nanos as f64 / 1_000_000.0;
        core::send_vsync(host_id, frame_time_ms);
    });
}

pub(crate) extern "system" fn setDisplayRefreshRate(
    _env: JNIEnv,
    _class: JClass,
    host_id: jint,
    refresh_period_nanos: jlong,
) {
    jni_safe!("setDisplayRefreshRate", {
        let _ = send_command_to_host(
            host_id,
            HostCommand::SetDisplayRefreshRate {
                period_nanos: refresh_period_nanos as i64,
            },
        );
    });
}

// ==================== Debug Stats ====================

pub(crate) extern "system" fn getDebugStats(
    env: JNIEnv,
    _class: JClass,
    host_id: jint,
) -> jni::sys::jbyteArray {
    jni_safe!("getDebugStats", std::ptr::null_mut(), {
        let stats = match shared::stats::get_stats(host_id) {
            Some(s) => s,
            None => return std::ptr::null_mut(),
        };

        let buf = stats.snapshot();

        match env.byte_array_from_slice(&buf) {
            Ok(arr) => arr.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    })
}

// ==================== Setting (Mode C) ====================

jni_json_callback!(onOpenSettingResult, "_internalOnOpenSettingResult");

// ==================== Share (Mode C) ====================

jni_json_callback!(onShareAppMessageResult, "_internalOnShareAppMessageResult");

// ==================== Navigate (Mode C) ====================

jni_json_callback!(
    onNavigateToMiniProgramResult,
    "_internalOnNavigateToMiniProgramResult"
);

// ==================== Payment (Mode C) ====================

jni_json_callback!(onMidasPaymentResult, "_internalOnMidasPaymentResult");
jni_json_callback!(
    onMidasPaymentGameItemResult,
    "_internalOnMidasPaymentGameItemResult"
);

// ==================== Video Callbacks ====================

pub(crate) extern "system" fn onVideoEvent<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    video_id: jint,
    event_type: JString<'local>,
    data_json: JString<'local>,
) {
    jni_safe!("onVideoEvent", {
        let evt: String = env
            .get_string(&event_type)
            .map(|s| s.into())
            .unwrap_or_default();
        let data: String = env
            .get_string(&data_json)
            .map(|s| s.into())
            .unwrap_or_else(|_| "{}".to_string());
        let _ = send_command_to_host(
            host_id,
            HostCommand::OnVideoStateChange {
                video_id: video_id as u32,
                event_type: evt,
                data,
            },
        );
    });
}

// ==================== Console Logs ====================

pub(crate) extern "system" fn getConsoleLogs<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    since_cursor: jlong,
) -> jstring {
    jni_safe!("getConsoleLogs", std::ptr::null_mut(), {
        let json = shared::console_log::read_console_logs_json(host_id, since_cursor as u64)
            .unwrap_or_else(|| r#"{"logs":[],"cursor":0}"#.to_string());
        match env.new_string(&json) {
            Ok(s) => s.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    })
}
