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

    /// Get the bounding client rect of the menu button (capsule) as JSON.
    ///
    /// Expected JSON fields: `width`, `height`, `top`, `bottom`, `left`, `right`.
    /// On platforms without a menu button, returns a reasonable default rect
    /// positioned at the top-right corner.
    fn get_menu_button_bounding_client_rect_json(&self) -> Result<String, String> {
        // Default: 87x32 rect at top-right, typical for mini-game capsule button.
        Ok(r#"{"width":87,"height":32,"top":4,"bottom":36,"left":278,"right":365}"#.to_string())
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

    /// Open the mini program setting page (Mode C, async).
    ///
    /// Result delivered via `onOpenSettingResult` callback with JSON:
    /// `{"authSetting":{"scope.userInfo":true,...}}`
    fn open_setting(&self, _options_json: &str) -> Result<(), String> {
        Err("openSetting:fail not supported".to_string())
    }

    /// Get app authorization setting as JSON string.
    ///
    /// Expected JSON fields: `albumAuthorized`, `bluetoothAuthorized`,
    /// `cameraAuthorized`, `locationAuthorized`, `microphoneAuthorized`, etc.
    fn get_app_authorization_setting_json(&self) -> Result<String, String> {
        Err("getAppAuthorizeSetting:fail not supported".to_string())
    }
}
