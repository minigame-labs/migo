use std::ffi::c_void;

use jni::{JNIEnv, NativeMethod};
use tracing::info;

use crate::android::jni::{
    JAVA_METHOD_CACHE, JavaMethodCache, init, mod_main, onAudioInterruptionBegin,
    onAudioInterruptionEnd, onHide, onModalResult, onActionSheetResult,
    onOpenAppAuthorizeSetting, onOpenSystemBluetoothSetting, onShow, onTouch, shutdown,
    updateSurface, version,
};

pub(crate) fn register_native_exports(env: &mut JNIEnv) -> Result<(), String> {
    let class = env
        .find_class("com/migo/runtime/internal/NativeBridge")
        .map_err(|e| format!("Failed to find NativeBridge: {e:?}"))?;

    env.register_native_methods(
        &class,
        &[
            NativeMethod {
                name: "version".into(),
                sig: "()Ljava/lang/String;".into(),
                fn_ptr: version as *mut c_void,
            },
            NativeMethod {
                name: "init".into(),
                sig: "(Ljava/lang/Object;Lcom/migo/runtime/RuntimeConfig;)I".into(),
                fn_ptr: init as *mut c_void,
            },
            NativeMethod {
                name: "shutdown".into(),
                sig: "(I)V".into(),
                fn_ptr: shutdown as *mut c_void,
            },
            NativeMethod {
                name: "onOpenSystemBluetoothSetting".into(),
                sig: "(II)V".into(),
                fn_ptr: onOpenSystemBluetoothSetting as *mut c_void,
            },
            NativeMethod {
                name: "onOpenAppAuthorizeSetting".into(),
                sig: "(II)V".into(),
                fn_ptr: onOpenAppAuthorizeSetting as *mut c_void,
            },
            NativeMethod {
                name: "onShow".into(),
                sig: "(I)V".into(),
                fn_ptr: onShow as *mut c_void,
            },
            NativeMethod {
                name: "onHide".into(),
                sig: "(I)V".into(),
                fn_ptr: onHide as *mut c_void,
            },
            NativeMethod {
                name: "onAudioInterruptionBegin".into(),
                sig: "(I)V".into(),
                fn_ptr: onAudioInterruptionBegin as *mut c_void,
            },
            NativeMethod {
                name: "onAudioInterruptionEnd".into(),
                sig: "(I)V".into(),
                fn_ptr: onAudioInterruptionEnd as *mut c_void,
            },
            NativeMethod {
                name: "updateSurface".into(),
                sig: "(ILjava/lang/Object;)V".into(),
                fn_ptr: updateSurface as *mut c_void,
            },
            NativeMethod {
                name: "onTouchEvent".into(),
                sig: "(IIJILjava/nio/ByteBuffer;)V".into(),
                fn_ptr: onTouch as *mut c_void,
            },
            NativeMethod {
                name: "modMain".into(),
                // (sessionId, gameId, entry) -> int
                sig: "(ILjava/lang/String;Ljava/lang/String;)I".into(),
                fn_ptr: mod_main as *mut c_void,
            },
            NativeMethod {
                name: "onModalResult".into(),
                sig: "(III)V".into(),
                fn_ptr: onModalResult as *mut c_void,
            },
            NativeMethod {
                name: "onActionSheetResult".into(),
                sig: "(II)V".into(),
                fn_ptr: onActionSheetResult as *mut c_void,
            },
        ],
    )
    .map_err(|e| format!("Failed to register native methods: {e:?}"))?;

    info!("Registered native methods for NativeBridge");
    Ok(())
}

pub(crate) fn register_java_exports(env: &mut JNIEnv) -> Result<(), String> {
    let local_class = env
        .find_class("com/migo/runtime/internal/NativeExports")
        .map_err(|e| format!("Failed to find class NativeExports: {e}"))?;

    let methods = [
        ("getCacheDirPath", "()Ljava/lang/String;"),
        ("openSystemBluetoothSetting", "(I)V"),
        ("openAppAuthorizeSetting", "(I)V"),
        ("getWindowInfoBytes", "(I)[B"),
        ("getSystemSettingInfoBytes", "()[B"),
        ("getDeviceInfoJson", "()Ljava/lang/String;"),
        ("getAppAuthorizationSettingJson", "()Ljava/lang/String;"),
        // UI interaction
        ("showToast", "(ILjava/lang/String;)V"),
        ("hideToast", "(I)V"),
        ("showModal", "(ILjava/lang/String;)V"),
        ("showLoading", "(ILjava/lang/String;)V"),
        ("hideLoading", "(I)V"),
        ("showActionSheet", "(ILjava/lang/String;)V"),
    ];

    let global_class = env
        .new_global_ref(local_class)
        .map_err(|e| format!("Failed to create global ref for NativeExports: {e}"))?;

    let mut cache = JavaMethodCache::new(global_class);

    for (name, sig) in methods {
        let mid = env
            .get_static_method_id("com/migo/runtime/internal/NativeExports", name, sig)
            .map_err(|e| format!("Failed to get method id for {name} {sig}: {e}"))?;
        cache.insert_method_id(name, mid);
    }

    JAVA_METHOD_CACHE
        .set(cache)
        .map_err(|_| "Failed to set JAVA_METHOD_CACHE (already initialized)".to_string())?;

    info!("Cached NativeExports class + static method IDs");
    Ok(())
}
