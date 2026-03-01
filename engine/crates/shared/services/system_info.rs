//! System information service traits for window info, device info, settings, and authorization.

use std::sync::Arc;

/// System information and settings service.
///
/// Provides cross-platform access to device info, window info, system settings,
/// and system-level operations like opening bluetooth/authorization settings.
pub trait SystemInfoService: Send + Sync {
    /// Open the system Bluetooth settings page.
    fn open_bluetooth_settings(&self) -> Result<(), String> {
        Err("openSystemBluetoothSetting:fail not supported".to_string())
    }

    /// Open the app authorization settings page.
    fn open_app_authorize_setting(&self) -> Result<(), String> {
        Err("openAppAuthorizeSetting:fail not supported".to_string())
    }

    /// Get window info as JSON string.
    ///
    /// Expected JSON fields: `pixel_ratio`, `screen_width`, `screen_height`,
    /// `window_width`, `window_height`, `status_bar_height`, `screen_top`,
    /// `safe_area: { left, top, right, bottom }`.
    fn get_window_info_json(&self) -> Result<String, String> {
        Err("getWindowInfo:fail not supported".to_string())
    }

    /// Get system settings as JSON string.
    ///
    /// Expected JSON fields: `bluetooth_enabled`, `location_enabled`,
    /// `wifi_enabled`, `orientation`.
    fn get_system_settings_json(&self) -> Result<String, String> {
        Err("getSystemSetting:fail not supported".to_string())
    }

    /// Get device info as JSON string.
    ///
    /// Expected JSON fields: `abi`, `deviceAbi`, `benchmarkLevel`, `brand`,
    /// `model`, `system`, `platform`, `cpuType`, `memorySize`.
    fn get_device_info_json(&self) -> Result<String, String> {
        Err("getDeviceInfo:fail not supported".to_string())
    }

    /// Get app authorization setting as JSON string.
    ///
    /// Expected JSON fields: `albumAuthorized`, `bluetoothAuthorized`,
    /// `cameraAuthorized`, `locationAuthorized`, `microphoneAuthorized`, etc.
    fn get_app_authorization_setting_json(&self) -> Result<String, String> {
        Err("getAppAuthorizeSetting:fail not supported".to_string())
    }
}
