//! Device service traits for sensors, battery, vibration, and screen.

use std::sync::Arc;

use super::{ClipboardService, NetworkService};

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
}
