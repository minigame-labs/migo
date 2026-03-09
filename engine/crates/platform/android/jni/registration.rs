use std::ffi::c_void;

use jni::{JNIEnv, NativeMethod};
use tracing::info;

use crate::android::jni::{
    JAVA_METHOD_CACHE, JavaMethodCache, init, mod_main, onAudioInterruptionBegin,
    onAudioInterruptionEnd, onAccelerometerChange, onCompassChange, onDeviceMotionChange,
    onDeviceOrientationChange, onGyroscopeChange, onHide, onModalResult, onActionSheetResult,
    onNetworkStatusChange, onOpenAppAuthorizeSetting, onOpenSystemBluetoothSetting, onShow,
    onRestart, onTouch, onVsync, getDebugStats, shutdown, updateSurface, version,
    onRecorderEvent, onRecorderFrameData,
    onCameraEvent, onCameraFrameData,
    onUserCaptureScreen,
    onBluetoothAdapterStateChange, onBluetoothDeviceFound,
    onBeaconUpdate, onBeaconServiceChange,
    onKeyboardInput, onKeyboardConfirm, onKeyboardComplete, onKeyboardHeightChange,
    onChooseImageResult, onChooseMessageFileResult,
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
                name: "onRestart".into(),
                sig: "(I)V".into(),
                fn_ptr: onRestart as *mut c_void,
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
            NativeMethod {
                name: "onDeviceMotionChange".into(),
                sig: "(IDDD)V".into(),
                fn_ptr: onDeviceMotionChange as *mut c_void,
            },
            NativeMethod {
                name: "onGyroscopeChange".into(),
                sig: "(IDDD)V".into(),
                fn_ptr: onGyroscopeChange as *mut c_void,
            },
            NativeMethod {
                name: "onDeviceOrientationChange".into(),
                sig: "(ILjava/lang/String;)V".into(),
                fn_ptr: onDeviceOrientationChange as *mut c_void,
            },
            NativeMethod {
                name: "onCompassChange".into(),
                sig: "(IDLjava/lang/String;)V".into(),
                fn_ptr: onCompassChange as *mut c_void,
            },
            NativeMethod {
                name: "onAccelerometerChange".into(),
                sig: "(IDDD)V".into(),
                fn_ptr: onAccelerometerChange as *mut c_void,
            },
            NativeMethod {
                name: "onNetworkStatusChange".into(),
                sig: "(IZLjava/lang/String;)V".into(),
                fn_ptr: onNetworkStatusChange as *mut c_void,
            },
            NativeMethod {
                name: "onVsync".into(),
                sig: "(IJ)V".into(),
                fn_ptr: onVsync as *mut c_void,
            },
            NativeMethod {
                name: "getDebugStats".into(),
                sig: "(I)[B".into(),
                fn_ptr: getDebugStats as *mut c_void,
            },
            NativeMethod {
                name: "onRecorderEvent".into(),
                sig: "(ILjava/lang/String;Ljava/lang/String;)V".into(),
                fn_ptr: onRecorderEvent as *mut c_void,
            },
            NativeMethod {
                name: "onRecorderFrameData".into(),
                sig: "(I[BZ)V".into(),
                fn_ptr: onRecorderFrameData as *mut c_void,
            },
            NativeMethod {
                name: "onCameraEvent".into(),
                sig: "(IILjava/lang/String;Ljava/lang/String;)V".into(),
                fn_ptr: onCameraEvent as *mut c_void,
            },
            NativeMethod {
                name: "onCameraFrameData".into(),
                sig: "(II[BII)V".into(),
                fn_ptr: onCameraFrameData as *mut c_void,
            },
            NativeMethod {
                name: "onUserCaptureScreen".into(),
                sig: "(I)V".into(),
                fn_ptr: onUserCaptureScreen as *mut c_void,
            },
            NativeMethod {
                name: "onBluetoothAdapterStateChange".into(),
                sig: "(IZZ)V".into(),
                fn_ptr: onBluetoothAdapterStateChange as *mut c_void,
            },
            NativeMethod {
                name: "onBluetoothDeviceFound".into(),
                sig: "(ILjava/lang/String;)V".into(),
                fn_ptr: onBluetoothDeviceFound as *mut c_void,
            },
            NativeMethod {
                name: "onBeaconUpdate".into(),
                sig: "(ILjava/lang/String;)V".into(),
                fn_ptr: onBeaconUpdate as *mut c_void,
            },
            NativeMethod {
                name: "onBeaconServiceChange".into(),
                sig: "(IZZ)V".into(),
                fn_ptr: onBeaconServiceChange as *mut c_void,
            },
            // Keyboard callbacks
            NativeMethod {
                name: "onKeyboardInput".into(),
                sig: "(ILjava/lang/String;)V".into(),
                fn_ptr: onKeyboardInput as *mut c_void,
            },
            NativeMethod {
                name: "onKeyboardConfirm".into(),
                sig: "(ILjava/lang/String;)V".into(),
                fn_ptr: onKeyboardConfirm as *mut c_void,
            },
            NativeMethod {
                name: "onKeyboardComplete".into(),
                sig: "(ILjava/lang/String;)V".into(),
                fn_ptr: onKeyboardComplete as *mut c_void,
            },
            NativeMethod {
                name: "onKeyboardHeightChange".into(),
                sig: "(ID)V".into(),
                fn_ptr: onKeyboardHeightChange as *mut c_void,
            },
            // Image API callbacks
            NativeMethod {
                name: "onChooseImageResult".into(),
                sig: "(ILjava/lang/String;)V".into(),
                fn_ptr: onChooseImageResult as *mut c_void,
            },
            NativeMethod {
                name: "onChooseMessageFileResult".into(),
                sig: "(ILjava/lang/String;)V".into(),
                fn_ptr: onChooseMessageFileResult as *mut c_void,
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
        // Battery
        ("getBatteryInfoJson", "()Ljava/lang/String;"),
        // Vibration
        ("vibrateShort", "(Ljava/lang/String;)I"),
        ("vibrateLong", "()I"),
        // Screen
        ("getScreenBrightness", "(I)F"),
        ("setScreenBrightness", "(IF)I"),
        ("setKeepScreenOn", "(IZ)I"),
        ("setDeviceOrientation", "(ILjava/lang/String;)I"),
        // Screen capture
        ("startCaptureScreen", "(I)V"),
        ("stopCaptureScreen", "(I)V"),
        // Device sensor
        ("startDeviceMotionListening", "(ILjava/lang/String;)V"),
        ("stopDeviceMotionListening", "(I)V"),
        ("startGyroscope", "(ILjava/lang/String;)V"),
        ("stopGyroscope", "(I)V"),
        // Compass
        ("startCompass", "(I)V"),
        ("stopCompass", "(I)V"),
        // Accelerometer
        ("startAccelerometer", "(ILjava/lang/String;)V"),
        ("stopAccelerometer", "(I)V"),
        // Network
        ("startNetworkMonitoring", "(I)V"),
        ("stopNetworkMonitoring", "(I)V"),
        ("getNetworkTypeJson", "(I)Ljava/lang/String;"),
        ("getLocalIPAddressJson", "()Ljava/lang/String;"),
        // File operations
        ("unzipFile", "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;"),
        // Charset encoding (GBK via java.nio.charset)
        ("encodeGbk", "(Ljava/lang/String;)[B"),
        ("decodeGbk", "([B)Ljava/lang/String;"),
        // Image decoding (BitmapFactory)
        ("decodeImageRgba", "([B)[B"),
        // Clipboard
        ("setClipboardData", "(ILjava/lang/String;)I"),
        ("getClipboardData", "(I)Ljava/lang/String;"),
        // Audio platform
        ("setInnerAudioOption", "(IZZZ)V"),
        ("getAvailableAudioSources", "(I)Ljava/lang/String;"),
        // Recorder
        ("recorderStart", "(ILjava/lang/String;)V"),
        ("recorderPause", "(I)V"),
        ("recorderResume", "(I)V"),
        ("recorderStop", "(I)V"),
        // Camera
        ("cameraCreate", "(ILjava/lang/String;)Ljava/lang/String;"),
        ("cameraDestroy", "(II)V"),
        ("cameraTakePhoto", "(ILjava/lang/String;)Ljava/lang/String;"),
        ("cameraStartRecord", "(ILjava/lang/String;)Ljava/lang/String;"),
        ("cameraStopRecord", "(ILjava/lang/String;)Ljava/lang/String;"),
        ("cameraSetZoom", "(ILjava/lang/String;)Ljava/lang/String;"),
        ("cameraListenFrameChange", "(II)V"),
        ("cameraCloseFrameChange", "(II)V"),
        // Bluetooth
        ("bluetoothOpenAdapter", "(ILjava/lang/String;)V"),
        ("bluetoothCloseAdapter", "(I)V"),
        ("bluetoothGetAdapterState", "(I)Ljava/lang/String;"),
        ("bluetoothStartDevicesDiscovery", "(ILjava/lang/String;)V"),
        ("bluetoothStopDevicesDiscovery", "(I)V"),
        ("bluetoothGetDevices", "(I)Ljava/lang/String;"),
        ("bluetoothGetConnectedDevices", "(ILjava/lang/String;)Ljava/lang/String;"),
        ("bluetoothMakePair", "(ILjava/lang/String;)V"),
        ("bluetoothIsDevicePaired", "(ILjava/lang/String;)V"),
        ("bluetoothStartBeaconDiscovery", "(ILjava/lang/String;)V"),
        ("bluetoothStopBeaconDiscovery", "(I)V"),
        ("bluetoothGetBeacons", "(I)Ljava/lang/String;"),
        // Keyboard
        ("keyboardShow", "(ILjava/lang/String;)V"),
        ("keyboardHide", "(I)V"),
        ("keyboardUpdate", "(ILjava/lang/String;)V"),
        // Image API
        ("imageSaveToPhotosAlbum", "(ILjava/lang/String;)V"),
        ("imagePreviewMedia", "(ILjava/lang/String;)V"),
        ("imagePreviewImage", "(ILjava/lang/String;)V"),
        ("imageCompress", "(ILjava/lang/String;)Ljava/lang/String;"),
        ("imageChooseMessageFile", "(ILjava/lang/String;)V"),
        ("imageChooseImage", "(ILjava/lang/String;)V"),
        // Error notification callback
        // onError(hostId, errorCode, message, detail)
        ("onError", "(IILjava/lang/String;Ljava/lang/String;)V"),
        // Exit callback
        ("onExit", "(I)V"),
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
