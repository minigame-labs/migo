//! Device service traits for sensors, battery, vibration, and screen.

use std::sync::Arc;

use super::{CameraService, ClipboardService, NetworkService};

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
}
