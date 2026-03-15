//! Device service traits for sensors, battery, vibration, and screen.

use std::sync::Arc;

use super::{
    CameraService, ClipboardService, CodecService, FileService, GameLogService, ImageApiService,
    InteractionService, LocationService, NetworkService, ScanCodeService, SystemInfoService,
};

// ==================== Battery ====================

/// Battery information service.
pub trait BatteryService: Send + Sync {
    /// Get battery info as JSON: `{"level": 80, "isCharging": true}`
    fn get_info_json(&self) -> Result<String, String> {
        Err("getBatteryInfo:fail not supported".to_string())
    }
}

// ==================== Vibration ====================

/// Vibration service for haptic feedback.
pub trait VibrationService: Send + Sync {
    /// Short vibration (15ms). type_: "heavy", "medium", "light"
    fn vibrate_short(&self, _type_: &str) -> Result<(), String> {
        Err("vibrateShort:fail not supported".to_string())
    }

    /// Long vibration (400ms).
    fn vibrate_long(&self) -> Result<(), String> {
        Err("vibrateLong:fail not supported".to_string())
    }
}

// ==================== Screen ====================

/// Screen brightness and orientation service.
pub trait ScreenService: Send + Sync {
    /// Get screen brightness (0.0-1.0).
    fn get_brightness(&self) -> Result<f32, String> {
        Err("getScreenBrightness:fail not supported".to_string())
    }

    /// Set screen brightness (0.0-1.0).
    fn set_brightness(&self, _value: f32) -> Result<(), String> {
        Err("setScreenBrightness:fail not supported".to_string())
    }

    /// Set keep screen on.
    fn set_keep_screen_on(&self, _keep_on: bool) -> Result<(), String> {
        Err("setKeepScreenOn:fail not supported".to_string())
    }

    /// Set device orientation: "portrait", "landscape", "landscapeReverse"
    fn set_orientation(&self, _value: &str) -> Result<(), String> {
        Err("setDeviceOrientation:fail not supported".to_string())
    }

    /// Start observing user screenshot events.
    fn start_capture_screen(&self) -> Result<(), String> {
        Err("onUserCaptureScreen:fail not supported".to_string())
    }

    /// Stop observing user screenshot events.
    fn stop_capture_screen(&self) -> Result<(), String> {
        Err("offUserCaptureScreen:fail not supported".to_string())
    }

    /// Set whether to enable debug mode at runtime.
    fn set_enable_debug(&self, _enabled: bool) -> Result<(), String> {
        Err("setEnableDebug:fail not supported".to_string())
    }
}

// ==================== Device Motion ====================

/// Device motion sensor (rotation angles).
pub trait DeviceMotionService: Send + Sync {
    /// Start listening. interval: "game" (20ms), "ui" (60ms), "normal" (200ms)
    fn start(&self, _interval: &str) -> Result<(), String> {
        Err("startDeviceMotionListening:fail not supported".to_string())
    }

    fn stop(&self) -> Result<(), String> {
        Err("stopDeviceMotionListening:fail not supported".to_string())
    }
}

// ==================== Gyroscope ====================

/// Gyroscope sensor (angular velocity rad/s).
pub trait GyroscopeService: Send + Sync {
    fn start(&self, _interval: &str) -> Result<(), String> {
        Err("startGyroscope:fail not supported".to_string())
    }

    fn stop(&self) -> Result<(), String> {
        Err("stopGyroscope:fail not supported".to_string())
    }
}

// ==================== Compass ====================

/// Compass sensor (magnetic heading degrees).
pub trait CompassService: Send + Sync {
    fn start(&self) -> Result<(), String> {
        Err("startCompass:fail not supported".to_string())
    }

    fn stop(&self) -> Result<(), String> {
        Err("stopCompass:fail not supported".to_string())
    }
}

// ==================== Accelerometer ====================

/// Accelerometer sensor (acceleration m/s²).
pub trait AccelerometerService: Send + Sync {
    fn start(&self, _interval: &str) -> Result<(), String> {
        Err("startAccelerometer:fail not supported".to_string())
    }

    fn stop(&self) -> Result<(), String> {
        Err("stopAccelerometer:fail not supported".to_string())
    }
}

// ==================== Audio Platform ====================

/// Audio platform service for device-level audio configuration.
///
/// Handles operations that require platform AudioManager access,
/// such as audio focus, speaker routing, and input source queries.
pub trait AudioPlatformService: Send + Sync {
    /// Configure inner audio behavior.
    ///
    /// - `mix_with_other`: Allow mixing with other audio apps (Android: abandon focus vs duck)
    /// - `obey_mute_switch`: Respect device mute/ringer mode
    /// - `speaker_on`: Route audio to speaker (vs earpiece/headset)
    fn set_inner_audio_option(
        &self,
        _mix_with_other: bool,
        _obey_mute_switch: bool,
        _speaker_on: bool,
    ) -> Result<(), String> {
        Err("setInnerAudioOption:fail not supported".to_string())
    }

    /// Get available audio input sources.
    ///
    /// Returns source identifiers matching `RecorderManager.start()` audioSource param.
    /// Maps to Android `MediaRecorder.AudioSource` constants:
    /// - "auto" (DEFAULT=0)
    /// - "buildInMic" (built-in microphone)
    /// - "headsetMic" (headset microphone, if connected)
    /// - "mic" (MIC=1)
    /// - "camcorder" (CAMCORDER=5)
    /// - "voice_recognition" (VOICE_RECOGNITION=6)
    /// - "voice_communication" (VOICE_COMMUNICATION=7)
    fn get_available_audio_sources(&self) -> Result<Vec<String>, String> {
        Err("getAvailableAudioSources:fail not supported".to_string())
    }
}

// ==================== Recorder ====================

/// Audio recording service (microphone input, encoding, file output).
///
/// Manages the platform media recorder lifecycle. Commands are fire-and-forget;
/// results and state changes are delivered asynchronously via `HostCommand::RecorderEvent`
/// and `HostCommand::RecorderFrameData`.
pub trait RecorderService: Send + Sync {
    /// Start recording with the given options (JSON-encoded).
    ///
    /// JSON fields:
    /// - `duration`: max recording duration in ms (default 60000, max 600000)
    /// - `sampleRate`: sample rate Hz (8000,11025,12000,16000,22050,24000,32000,44100,48000)
    /// - `numberOfChannels`: 1 or 2 (default 2)
    /// - `encodeBitRate`: encode bit rate in bps (default 48000)
    /// - `format`: "mp3", "aac", "wav", "PCM" (default "aac")
    /// - `frameSize`: frame size in KB (if set, triggers onFrameRecorded)
    /// - `audioSource`: "auto","buildInMic","headsetMic","mic","camcorder",
    ///                   "voice_recognition","voice_communication" (default "auto")
    fn start(&self, _options_json: &str) -> Result<(), String> {
        Err("recorderManager.start:fail not supported".to_string())
    }

    /// Pause recording.
    fn pause(&self) -> Result<(), String> {
        Err("recorderManager.pause:fail not supported".to_string())
    }

    /// Resume recording after pause.
    fn resume(&self) -> Result<(), String> {
        Err("recorderManager.resume:fail not supported".to_string())
    }

    /// Stop recording. The platform will fire a RecorderEvent("stop", ...) with the file path.
    fn stop(&self) -> Result<(), String> {
        Err("recorderManager.stop:fail not supported".to_string())
    }
}

// ==================== Keyboard ====================

/// Soft keyboard service for text input.
pub trait KeyboardService: Send + Sync {
    /// Show the soft keyboard with options (JSON-encoded).
    ///
    /// JSON fields:
    /// - `defaultValue`: default text value
    /// - `maxLength`: max input length
    /// - `multiple`: multi-line input
    /// - `confirmHold`: keep keyboard on confirm
    /// - `confirmType`: confirm button type ("done","next","search","go","send")
    /// - `keyboardType`: keyboard type ("text","number")
    fn show(&self, _options_json: &str) -> Result<(), String> {
        Err("showKeyboard:fail not supported".to_string())
    }

    /// Hide the soft keyboard.
    fn hide(&self) -> Result<(), String> {
        Err("hideKeyboard:fail not supported".to_string())
    }

    /// Update the keyboard input value.
    fn update(&self, _value: &str) -> Result<(), String> {
        Err("updateKeyboard:fail not supported".to_string())
    }
}

// ==================== Bluetooth ====================

/// Bluetooth service for BLE and Beacon operations.
pub trait BluetoothService: Send + Sync {
    /// Initialize the Bluetooth adapter.
    ///
    /// JSON fields:
    /// - `mode`: "central" (default) or "peripheral" (iOS only)
    fn open_adapter(&self, _options_json: &str) -> Result<(), String> {
        Err("openBluetoothAdapter:fail not supported".to_string())
    }

    /// Close the Bluetooth adapter and release resources.
    fn close_adapter(&self) -> Result<(), String> {
        Err("closeBluetoothAdapter:fail not supported".to_string())
    }

    /// Get Bluetooth adapter state.
    /// Returns JSON: `{"discovering": bool, "available": bool}`
    fn get_adapter_state(&self) -> Result<String, String> {
        Err("getBluetoothAdapterState:fail not supported".to_string())
    }

    /// Start scanning for BLE devices.
    ///
    /// JSON fields:
    /// - `services`: array of service UUID strings to filter
    /// - `allowDuplicatesKey`: bool (default false)
    /// - `interval`: number in ms (default 0)
    /// - `powerLevel`: "low" | "medium" | "high" (default "medium")
    fn start_devices_discovery(&self, _options_json: &str) -> Result<(), String> {
        Err("startBluetoothDevicesDiscovery:fail not supported".to_string())
    }

    /// Stop scanning for BLE devices.
    fn stop_devices_discovery(&self) -> Result<(), String> {
        Err("stopBluetoothDevicesDiscovery:fail not supported".to_string())
    }

    /// Get all discovered Bluetooth devices.
    /// Returns JSON: `{"devices": [...]}`
    fn get_devices(&self) -> Result<String, String> {
        Err("getBluetoothDevices:fail not supported".to_string())
    }

    /// Get connected Bluetooth devices by service UUIDs.
    ///
    /// JSON fields:
    /// - `services`: array of service UUID strings
    ///
    /// Returns JSON: `{"devices": [...]}`
    fn get_connected_devices(&self, _options_json: &str) -> Result<String, String> {
        Err("getConnectedBluetoothDevices:fail not supported".to_string())
    }

    /// Pair with a Bluetooth device (Android only).
    ///
    /// JSON fields:
    /// - `deviceId`: string
    /// - `pin`: string (Base64)
    /// - `timeout`: number in ms (default 20000)
    fn make_pair(&self, _options_json: &str) -> Result<(), String> {
        Err("makeBluetoothPair:fail not supported".to_string())
    }

    /// Check if a Bluetooth device is paired (Android only).
    ///
    /// JSON fields:
    /// - `deviceId`: string
    fn is_device_paired(&self, _options_json: &str) -> Result<(), String> {
        Err("isBluetoothDevicePaired:fail not supported".to_string())
    }

    /// Start Beacon discovery.
    ///
    /// JSON fields:
    /// - `uuids`: array of UUID strings
    /// - `ignoreBluetoothAvailable`: bool (default false)
    fn start_beacon_discovery(&self, _options_json: &str) -> Result<(), String> {
        Err("startBeaconDiscovery:fail not supported".to_string())
    }

    /// Stop Beacon discovery.
    fn stop_beacon_discovery(&self) -> Result<(), String> {
        Err("stopBeaconDiscovery:fail not supported".to_string())
    }

    /// Get all discovered Beacon devices.
    /// Returns JSON: `{"beacons": [...]}`
    fn get_beacons(&self) -> Result<String, String> {
        Err("getBeacons:fail not supported".to_string())
    }
}

// ==================== Aggregated Device Services ====================

/// Aggregated device services provided by a platform.
///
/// Platforms implement this trait to provide all device capabilities.
pub trait DeviceServices: Send + Sync {
    fn clipboard(&self) -> Option<Arc<dyn ClipboardService>> {
        None
    }
    fn battery(&self) -> Option<Arc<dyn BatteryService>> {
        None
    }
    fn vibration(&self) -> Option<Arc<dyn VibrationService>> {
        None
    }
    fn screen(&self) -> Option<Arc<dyn ScreenService>> {
        None
    }
    fn device_motion(&self) -> Option<Arc<dyn DeviceMotionService>> {
        None
    }
    fn gyroscope(&self) -> Option<Arc<dyn GyroscopeService>> {
        None
    }
    fn compass(&self) -> Option<Arc<dyn CompassService>> {
        None
    }
    fn accelerometer(&self) -> Option<Arc<dyn AccelerometerService>> {
        None
    }
    fn network(&self) -> Option<Arc<dyn NetworkService>> {
        None
    }
    fn audio_platform(&self) -> Option<Arc<dyn AudioPlatformService>> {
        None
    }
    fn recorder(&self) -> Option<Arc<dyn RecorderService>> {
        None
    }
    fn camera(&self) -> Option<Arc<dyn CameraService>> {
        None
    }
    fn interaction(&self) -> Option<Arc<dyn InteractionService>> {
        None
    }
    fn system_info(&self) -> Option<Arc<dyn SystemInfoService>> {
        None
    }
    fn codec(&self) -> Option<Arc<dyn CodecService>> {
        None
    }
    fn file(&self) -> Option<Arc<dyn FileService>> {
        None
    }
    fn keyboard(&self) -> Option<Arc<dyn KeyboardService>> {
        None
    }
    fn bluetooth(&self) -> Option<Arc<dyn BluetoothService>> {
        None
    }
    fn image_api(&self) -> Option<Arc<dyn ImageApiService>> {
        None
    }
    fn location(&self) -> Option<Arc<dyn LocationService>> {
        None
    }
    fn scan_code(&self) -> Option<Arc<dyn ScanCodeService>> {
        None
    }
    fn game_log(&self) -> Option<Arc<dyn GameLogService>> {
        None
    }
}
