#![allow(non_snake_case)]
#![allow(dead_code)]

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::Arc;

use jni::objects::{JByteBuffer, JClass, JObject, JString};
use jni::sys::{jint, jlong, jobject, jstring};
use jni::{JNIEnv, JavaVM};

use smallvec::SmallVec;
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

    let app_tmp_dir = match super::get_string_field(&mut env, "appTmpDir", &options) {
        Ok(s) => s,
        Err(e) => {
            error!("init failed: read appTmpDir error: {}", e);
            return -1;
        }
    };

    let dpi = match super::get_f32(&mut env, "dpi", &options) {
        Ok(v) => v,
        Err(e) => {
            error!("init failed: read dpi error: {}", e);
            return -1;
        }
    };

    let init_options = InitOptions::new()
        .with_pixel_ratio(dpi)
        .with_tmp_dir(PathBuf::from(app_tmp_dir));

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

pub(crate) extern "system" fn onUnzipDone<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    host_id: jint,
    request_id: jint,
) {
    let cmd = HostCommand::EvalScript {
        source: format!("_internalOnUnZipDone({});", request_id),
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
    if count <= 0 {
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

    // Validate buffer capacity before creating slice
    let capacity = match env.get_direct_buffer_capacity(&buf) {
        Ok(cap) => cap,
        Err(e) => {
            error!("onTouch failed: get_direct_buffer_capacity error: {:?}", e);
            return;
        }
    };

    let expected_size = (count as usize) * std::mem::size_of::<TouchPoint>();
    if expected_size > capacity {
        error!(
            "onTouch failed: buffer underflow - expected {} bytes, got {} bytes",
            expected_size, capacity
        );
        return;
    }

    // SAFETY: We have verified that:
    // 1. addr is a valid pointer from get_direct_buffer_address
    // 2. The buffer has sufficient capacity for `count` TouchPoints
    // 3. TouchPoint memory layout matches Java side packing (repr(C))
    let slice = unsafe { std::slice::from_raw_parts(addr as *const TouchPoint, count as usize) };
    let points: SmallVec<[TouchPoint; 8]> = slice.iter().copied().collect();

    let touch_type = match action {
        0 | 5 => TouchType::Start,
        1 | 6 => TouchType::End,
        2 => TouchType::Move,
        3 => TouchType::Cancel,
        _ => TouchType::Move,
    };

    let cmd = HostCommand::OnTouch {
        touch_type,
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
    code_dir: JString<'local>,
    entry: JString<'local>,
) -> jint {
    let code_dir = match env.get_string(&code_dir) {
        Ok(s) => s.to_string_lossy().into_owned(),
        Err(e) => {
            error!("modMain failed: convert code_dir JString error: {:?}", e);
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

    match send_command_to_host(
        host_id,
        HostCommand::EvaluateModule {
            dir: code_dir,
            entry,
        },
    ) {
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
