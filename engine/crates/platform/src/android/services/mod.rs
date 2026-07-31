//! Android device service implementations.
//!
//! Implements `migo_core::services::DeviceServices` traits using JNI calls.

use std::sync::Arc;

use migo_core::services::{
    AccelerometerService, AdService, AudioPlatformService, AuthService, BatteryService, BluetoothService,
    CameraService, ClipboardService, CodecService, CommerceServices, CompassService,
    ConnectivityServices, DeviceMotionService, FileService, GameLogService, GyroscopeService,
    ImageApiService, InteractionService, KeyboardService, LocationService, MediaServices,
    NavigateService, NetworkService, PaymentService, RecorderService, ScanCodeService,
    ScreenService, SensorServices, ServiceError, ShareService, SubpackageService,
    SystemInfoService, SystemUtilServices, VibrationService, VideoService,
};

use crate::android::jni;

/// Android device services aggregator.
pub struct AndroidDeviceServices {
    host_id: i32,
    /// Set when an embedding host services the keyboard itself; see
    /// `AndroidPlatform::with_host_keyboard`.
    host_keyboard: Option<Arc<dyn KeyboardService>>,
}

impl AndroidDeviceServices {
    pub fn new(host_id: i32) -> Self {
        Self::with_host_keyboard(host_id, None)
    }

    pub fn with_host_keyboard(
        host_id: i32,
        host_keyboard: Option<Arc<dyn KeyboardService>>,
    ) -> Self {
        Self {
            host_id,
            host_keyboard,
        }
    }
}

// ---- SensorServices ----
impl SensorServices for AndroidDeviceServices {
    #[cfg(feature = "api-sensors")]
    fn battery(&self) -> Option<Arc<dyn BatteryService>> {
        Some(Arc::new(AndroidBattery))
    }
    #[cfg(feature = "api-sensors")]
    fn vibration(&self) -> Option<Arc<dyn VibrationService>> {
        Some(Arc::new(AndroidVibration))
    }
    #[cfg(feature = "api-sensors")]
    fn screen(&self) -> Option<Arc<dyn ScreenService>> {
        Some(Arc::new(AndroidScreen {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-sensors")]
    fn device_motion(&self) -> Option<Arc<dyn DeviceMotionService>> {
        Some(Arc::new(AndroidDeviceMotion {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-sensors")]
    fn gyroscope(&self) -> Option<Arc<dyn GyroscopeService>> {
        Some(Arc::new(AndroidGyroscope {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-sensors")]
    fn compass(&self) -> Option<Arc<dyn CompassService>> {
        Some(Arc::new(AndroidCompass {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-sensors")]
    fn accelerometer(&self) -> Option<Arc<dyn AccelerometerService>> {
        Some(Arc::new(AndroidAccelerometer {
            host_id: self.host_id,
        }))
    }
}

// ---- MediaServices ----
impl MediaServices for AndroidDeviceServices {
    #[cfg(feature = "api-media")]
    fn audio_platform(&self) -> Option<Arc<dyn AudioPlatformService>> {
        Some(Arc::new(AndroidAudioPlatform {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-media")]
    fn recorder(&self) -> Option<Arc<dyn RecorderService>> {
        Some(Arc::new(AndroidRecorder {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-media")]
    fn camera(&self) -> Option<Arc<dyn CameraService>> {
        Some(Arc::new(AndroidCamera {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-media")]
    fn image_api(&self) -> Option<Arc<dyn ImageApiService>> {
        Some(Arc::new(AndroidImageApi {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-media")]
    fn video(&self) -> Option<Arc<dyn VideoService>> {
        Some(Arc::new(AndroidVideo {
            host_id: self.host_id,
        }))
    }
}

// ---- ConnectivityServices ----
impl ConnectivityServices for AndroidDeviceServices {
    #[cfg(feature = "api-sensors")]
    fn network(&self) -> Option<Arc<dyn NetworkService>> {
        Some(Arc::new(AndroidNetwork {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-connectivity")]
    fn bluetooth(&self) -> Option<Arc<dyn BluetoothService>> {
        Some(Arc::new(AndroidBluetooth {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-sensors")]
    fn location(&self) -> Option<Arc<dyn LocationService>> {
        Some(Arc::new(AndroidLocation {
            host_id: self.host_id,
        }))
    }
}

// ---- CommerceServices ----
impl CommerceServices for AndroidDeviceServices {
    #[cfg(feature = "api-connectivity")]
    fn game_log(&self) -> Option<Arc<dyn GameLogService>> {
        Some(Arc::new(AndroidGameLog {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-connectivity")]
    fn auth(&self) -> Option<Arc<dyn AuthService>> {
        Some(Arc::new(AndroidAuth {
            host_id: self.host_id,
        }))
    }
    fn subpackage(&self) -> Option<Arc<dyn SubpackageService>> {
        Some(Arc::new(AndroidSubpackage {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-commerce")]
    fn share(&self) -> Option<Arc<dyn ShareService>> {
        Some(Arc::new(AndroidShare {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-commerce")]
    fn payment(&self) -> Option<Arc<dyn PaymentService>> {
        Some(Arc::new(AndroidPayment {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-commerce")]
    fn ad(&self) -> Option<Arc<dyn AdService>> {
        Some(Arc::new(AndroidAd {
            host_id: self.host_id,
        }))
    }
}

// ---- SystemUtilServices ----
impl SystemUtilServices for AndroidDeviceServices {
    #[cfg(feature = "api-sensors")]
    fn clipboard(&self) -> Option<Arc<dyn ClipboardService>> {
        Some(Arc::new(AndroidClipboard {
            host_id: self.host_id,
        }))
    }
    fn keyboard(&self) -> Option<Arc<dyn KeyboardService>> {
        // The host's own comes first: `AndroidKeyboard` reaches the Java SDK
        // over JNI, which a pure-native host has not got, and this accessor
        // would otherwise claim a capability it cannot deliver.
        if let Some(host_keyboard) = &self.host_keyboard {
            return Some(Arc::clone(host_keyboard));
        }
        Some(Arc::new(AndroidKeyboard {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-system")]
    fn interaction(&self) -> Option<Arc<dyn InteractionService>> {
        Some(Arc::new(AndroidInteraction {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-connectivity")]
    fn system_info(&self) -> Option<Arc<dyn SystemInfoService>> {
        Some(Arc::new(AndroidSystemInfo {
            host_id: self.host_id,
        }))
    }
    fn codec(&self) -> Option<Arc<dyn CodecService>> {
        Some(Arc::new(AndroidCodec))
    }
    fn file(&self) -> Option<Arc<dyn FileService>> {
        Some(Arc::new(AndroidFile))
    }
    #[cfg(feature = "api-sensors")]
    fn scan_code(&self) -> Option<Arc<dyn ScanCodeService>> {
        Some(Arc::new(AndroidScanCode {
            host_id: self.host_id,
        }))
    }
    #[cfg(feature = "api-connectivity")]
    fn navigate(&self) -> Option<Arc<dyn NavigateService>> {
        Some(Arc::new(AndroidNavigate {
            host_id: self.host_id,
        }))
    }
}

// DeviceServices is auto-implemented via blanket impl in shared::services::device

// ==================== Clipboard ====================

struct AndroidClipboard {
    host_id: i32,
}

impl ClipboardService for AndroidClipboard {
    fn set_data(&self, data: &str) -> Result<(), ServiceError> {
        Ok(jni::set_clipboard_data(self.host_id, data)?)
    }

    fn get_data(&self) -> Result<String, ServiceError> {
        Ok(jni::get_clipboard_data(self.host_id)?)
    }
}

// ==================== Battery ====================

struct AndroidBattery;

impl BatteryService for AndroidBattery {
    fn get_info_json(&self) -> Result<String, ServiceError> {
        Ok(jni::get_battery_info_json()?)
    }
}

// ==================== Vibration ====================

struct AndroidVibration;

impl VibrationService for AndroidVibration {
    fn vibrate_short(&self, type_: &str) -> Result<(), ServiceError> {
        jni::vibrate_short(type_).map(|_| ()).map_err(Into::into)
    }

    fn vibrate_long(&self) -> Result<(), ServiceError> {
        jni::vibrate_long().map(|_| ()).map_err(Into::into)
    }
}

// ==================== Screen ====================

struct AndroidScreen {
    host_id: i32,
}

impl ScreenService for AndroidScreen {
    fn get_brightness(&self) -> Result<f32, ServiceError> {
        Ok(jni::get_screen_brightness(self.host_id)?)
    }

    fn set_brightness(&self, value: f32) -> Result<(), ServiceError> {
        jni::set_screen_brightness(self.host_id, value)
            .map(|_| ())
            .map_err(Into::into)
    }

    fn set_keep_screen_on(&self, keep_on: bool) -> Result<(), ServiceError> {
        jni::set_keep_screen_on(self.host_id, keep_on)
            .map(|_| ())
            .map_err(Into::into)
    }

    fn set_orientation(&self, value: &str) -> Result<(), ServiceError> {
        jni::set_device_orientation(self.host_id, value)
            .map(|_| ())
            .map_err(Into::into)
    }

    fn start_capture_screen(&self) -> Result<(), ServiceError> {
        Ok(jni::start_capture_screen(self.host_id)?)
    }

    fn stop_capture_screen(&self) -> Result<(), ServiceError> {
        Ok(jni::stop_capture_screen(self.host_id)?)
    }

    fn set_enable_debug(&self, enabled: bool) -> Result<(), ServiceError> {
        jni::set_enable_debug(self.host_id, enabled)
            .map(|_| ())
            .map_err(Into::into)
    }
}

// ==================== Device Motion ====================

struct AndroidDeviceMotion {
    host_id: i32,
}

impl DeviceMotionService for AndroidDeviceMotion {
    fn start(&self, interval: &str) -> Result<(), ServiceError> {
        Ok(jni::start_device_motion(self.host_id, interval)?)
    }

    fn stop(&self) -> Result<(), ServiceError> {
        Ok(jni::stop_device_motion(self.host_id)?)
    }
}

// ==================== Gyroscope ====================

struct AndroidGyroscope {
    host_id: i32,
}

impl GyroscopeService for AndroidGyroscope {
    fn start(&self, interval: &str) -> Result<(), ServiceError> {
        Ok(jni::start_gyroscope(self.host_id, interval)?)
    }

    fn stop(&self) -> Result<(), ServiceError> {
        Ok(jni::stop_gyroscope(self.host_id)?)
    }
}

// ==================== Compass ====================

struct AndroidCompass {
    host_id: i32,
}

impl CompassService for AndroidCompass {
    fn start(&self) -> Result<(), ServiceError> {
        Ok(jni::start_compass(self.host_id)?)
    }

    fn stop(&self) -> Result<(), ServiceError> {
        Ok(jni::stop_compass(self.host_id)?)
    }
}

// ==================== Accelerometer ====================

struct AndroidAccelerometer {
    host_id: i32,
}

impl AccelerometerService for AndroidAccelerometer {
    fn start(&self, interval: &str) -> Result<(), ServiceError> {
        Ok(jni::start_accelerometer(self.host_id, interval)?)
    }

    fn stop(&self) -> Result<(), ServiceError> {
        Ok(jni::stop_accelerometer(self.host_id)?)
    }
}

// ==================== Network ====================

struct AndroidNetwork {
    host_id: i32,
}

impl NetworkService for AndroidNetwork {
    fn start_monitoring(&self) -> Result<(), ServiceError> {
        Ok(jni::start_network_monitoring(self.host_id)?)
    }

    fn stop_monitoring(&self) -> Result<(), ServiceError> {
        Ok(jni::stop_network_monitoring(self.host_id)?)
    }

    fn get_network_type_json(&self) -> Result<String, ServiceError> {
        Ok(jni::get_network_type_json(self.host_id)?)
    }

    fn get_local_ip_json(&self) -> Result<String, ServiceError> {
        Ok(jni::get_local_ip_address_json()?)
    }
}

// ==================== Audio Platform ====================

struct AndroidAudioPlatform {
    host_id: i32,
}

impl AudioPlatformService for AndroidAudioPlatform {
    fn set_inner_audio_option(
        &self,
        mix_with_other: bool,
        obey_mute_switch: bool,
        speaker_on: bool,
    ) -> Result<(), ServiceError> {
        Ok(jni::set_inner_audio_option(
            self.host_id,
            mix_with_other,
            obey_mute_switch,
            speaker_on,
        )?)
    }

    fn get_available_audio_sources(&self) -> Result<Vec<String>, ServiceError> {
        let csv = jni::get_available_audio_sources(self.host_id)?;
        // Parse comma-separated list: "auto,mic,camcorder,voice_recognition,voice_communication"
        let sources: Vec<String> = csv
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Ok(sources)
    }
}

// ==================== Recorder ====================

struct AndroidRecorder {
    host_id: i32,
}

impl RecorderService for AndroidRecorder {
    fn start(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::recorder_start(self.host_id, options_json)?)
    }

    fn pause(&self) -> Result<(), ServiceError> {
        Ok(jni::recorder_pause(self.host_id)?)
    }

    fn resume(&self) -> Result<(), ServiceError> {
        Ok(jni::recorder_resume(self.host_id)?)
    }

    fn stop(&self) -> Result<(), ServiceError> {
        Ok(jni::recorder_stop(self.host_id)?)
    }
}

// ==================== Camera ====================

struct AndroidCamera {
    host_id: i32,
}

impl CameraService for AndroidCamera {
    fn create(&self, options_json: &str) -> Result<String, ServiceError> {
        Ok(jni::camera_create(self.host_id, options_json)?)
    }

    fn destroy(&self, camera_id: u32) -> Result<(), ServiceError> {
        Ok(jni::camera_destroy(self.host_id, camera_id)?)
    }

    fn take_photo(&self, options_json: &str) -> Result<String, ServiceError> {
        Ok(jni::camera_take_photo(self.host_id, options_json)?)
    }

    fn start_record(&self, options_json: &str) -> Result<String, ServiceError> {
        Ok(jni::camera_start_record(self.host_id, options_json)?)
    }

    fn stop_record(&self, options_json: &str) -> Result<String, ServiceError> {
        Ok(jni::camera_stop_record(self.host_id, options_json)?)
    }

    fn set_zoom(&self, options_json: &str) -> Result<String, ServiceError> {
        Ok(jni::camera_set_zoom(self.host_id, options_json)?)
    }

    fn listen_frame_change(&self, camera_id: u32) -> Result<(), ServiceError> {
        Ok(jni::camera_listen_frame_change(self.host_id, camera_id)?)
    }

    fn close_frame_change(&self, camera_id: u32) -> Result<(), ServiceError> {
        Ok(jni::camera_close_frame_change(self.host_id, camera_id)?)
    }
}

// ==================== UI Interaction ====================

struct AndroidInteraction {
    host_id: i32,
}

impl InteractionService for AndroidInteraction {
    fn show_toast(&self, json: &str) -> Result<(), ServiceError> {
        Ok(jni::show_toast(self.host_id, json)?)
    }

    fn hide_toast(&self) -> Result<(), ServiceError> {
        Ok(jni::hide_toast(self.host_id)?)
    }

    fn show_modal(&self, json: &str) -> Result<(), ServiceError> {
        Ok(jni::show_modal(self.host_id, json)?)
    }

    fn show_loading(&self, json: &str) -> Result<(), ServiceError> {
        Ok(jni::show_loading(self.host_id, json)?)
    }

    fn hide_loading(&self) -> Result<(), ServiceError> {
        Ok(jni::hide_loading(self.host_id)?)
    }

    fn show_action_sheet(&self, json: &str) -> Result<(), ServiceError> {
        Ok(jni::show_action_sheet(self.host_id, json)?)
    }
}

// ==================== System Info ====================

struct AndroidSystemInfo {
    host_id: i32,
}

impl SystemInfoService for AndroidSystemInfo {
    fn open_bluetooth_settings(&self) -> Result<(), ServiceError> {
        Ok(jni::open_bluetooth_settings(self.host_id)?)
    }

    fn open_app_authorize_setting(&self) -> Result<(), ServiceError> {
        Ok(jni::open_app_authorize_setting(self.host_id)?)
    }

    fn get_window_info_json(&self) -> Result<String, ServiceError> {
        let info = jni::get_window_info(self.host_id)?;
        serde_json::to_string(&info)
            .map_err(|e| ServiceError::system(format!("getWindowInfo:fail {}", e)))
    }

    fn get_system_settings_json(&self) -> Result<String, ServiceError> {
        let settings = jni::get_system_settings()?;
        serde_json::to_string(&settings)
            .map_err(|e| ServiceError::system(format!("getSystemSetting:fail {}", e)))
    }

    fn get_device_info_json(&self) -> Result<String, ServiceError> {
        Ok(jni::get_device_info_json()?)
    }

    fn get_app_authorization_setting_json(&self) -> Result<String, ServiceError> {
        Ok(jni::get_app_authorization_setting_json()?)
    }

    fn open_setting(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::open_setting(self.host_id, options_json)?)
    }
}

// ==================== Bluetooth ====================

struct AndroidBluetooth {
    host_id: i32,
}

impl BluetoothService for AndroidBluetooth {
    fn open_adapter(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::bluetooth_open_adapter(self.host_id, options_json)?)
    }

    fn close_adapter(&self) -> Result<(), ServiceError> {
        Ok(jni::bluetooth_close_adapter(self.host_id)?)
    }

    fn get_adapter_state(&self) -> Result<String, ServiceError> {
        Ok(jni::bluetooth_get_adapter_state(self.host_id)?)
    }

    fn start_devices_discovery(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::bluetooth_start_devices_discovery(
            self.host_id,
            options_json,
        )?)
    }

    fn stop_devices_discovery(&self) -> Result<(), ServiceError> {
        Ok(jni::bluetooth_stop_devices_discovery(self.host_id)?)
    }

    fn get_devices(&self) -> Result<String, ServiceError> {
        Ok(jni::bluetooth_get_devices(self.host_id)?)
    }

    fn get_connected_devices(&self, options_json: &str) -> Result<String, ServiceError> {
        Ok(jni::bluetooth_get_connected_devices(
            self.host_id,
            options_json,
        )?)
    }

    fn make_pair(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::bluetooth_make_pair(self.host_id, options_json)?)
    }

    fn is_device_paired(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::bluetooth_is_device_paired(self.host_id, options_json)?)
    }

    fn start_beacon_discovery(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::bluetooth_start_beacon_discovery(
            self.host_id,
            options_json,
        )?)
    }

    fn stop_beacon_discovery(&self) -> Result<(), ServiceError> {
        Ok(jni::bluetooth_stop_beacon_discovery(self.host_id)?)
    }

    fn get_beacons(&self) -> Result<String, ServiceError> {
        Ok(jni::bluetooth_get_beacons(self.host_id)?)
    }

    // ---- BLE GATT ----

    fn create_ble_connection(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::ble_create_connection(self.host_id, options_json)?)
    }

    fn close_ble_connection(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::ble_close_connection(self.host_id, options_json)?)
    }

    fn get_ble_device_services(&self, options_json: &str) -> Result<String, ServiceError> {
        Ok(jni::ble_get_device_services(self.host_id, options_json)?)
    }

    fn get_ble_device_characteristics(&self, options_json: &str) -> Result<String, ServiceError> {
        Ok(jni::ble_get_device_characteristics(
            self.host_id,
            options_json,
        )?)
    }

    fn read_ble_characteristic_value(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::ble_read_characteristic_value(
            self.host_id,
            options_json,
        )?)
    }

    fn write_ble_characteristic_value(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::ble_write_characteristic_value(
            self.host_id,
            options_json,
        )?)
    }

    fn notify_ble_characteristic_value_change(
        &self,
        options_json: &str,
    ) -> Result<(), ServiceError> {
        Ok(jni::ble_notify_characteristic_value_change(
            self.host_id,
            options_json,
        )?)
    }

    fn get_ble_device_rssi(&self, options_json: &str) -> Result<String, ServiceError> {
        Ok(jni::ble_get_device_rssi(self.host_id, options_json)?)
    }

    fn set_ble_mtu(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::ble_set_mtu(self.host_id, options_json)?)
    }

    fn get_ble_mtu(&self, options_json: &str) -> Result<String, ServiceError> {
        Ok(jni::ble_get_mtu(self.host_id, options_json)?)
    }
}

// ==================== Keyboard ====================

struct AndroidKeyboard {
    host_id: i32,
}

impl KeyboardService for AndroidKeyboard {
    fn show(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::keyboard_show(self.host_id, options_json)?)
    }

    fn hide(&self) -> Result<(), ServiceError> {
        Ok(jni::keyboard_hide(self.host_id)?)
    }

    fn update(&self, value: &str) -> Result<(), ServiceError> {
        Ok(jni::keyboard_update(self.host_id, value)?)
    }
}

// ==================== Codec (GBK) ====================

struct AndroidCodec;

impl CodecService for AndroidCodec {
    fn encode_gbk(&self, data: &str) -> Result<Vec<u8>, ServiceError> {
        Ok(jni::outbound::encode_gbk(data)?)
    }

    fn decode_gbk(&self, data: &[u8]) -> Result<String, ServiceError> {
        Ok(jni::outbound::decode_gbk(data)?)
    }
}

// ==================== File (Unzip) ====================

struct AndroidFile;

impl FileService for AndroidFile {
    fn unzip(&self, zip_path: &str, dest_dir: &str) -> Result<usize, ServiceError> {
        Ok(jni::outbound::unzip_file(zip_path, dest_dir)?)
    }
}

// ==================== Image API ====================

struct AndroidImageApi {
    host_id: i32,
}

impl ImageApiService for AndroidImageApi {
    fn save_image_to_photos_album(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::image_save_to_photos_album(self.host_id, options_json)?)
    }

    fn preview_media(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::image_preview_media(self.host_id, options_json)?)
    }

    fn preview_image(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::image_preview_image(self.host_id, options_json)?)
    }

    fn compress_image(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::image_compress(self.host_id, options_json)?)
    }

    fn choose_message_file(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::image_choose_message_file(self.host_id, options_json)?)
    }

    fn choose_image(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::image_choose_image(self.host_id, options_json)?)
    }
}

// ==================== Video ====================

struct AndroidVideo {
    host_id: i32,
}

impl VideoService for AndroidVideo {
    fn create(&self, options_json: &str) -> Result<String, ServiceError> {
        Ok(jni::video_create(self.host_id, options_json)?)
    }
    fn play(&self, video_id: u32) -> Result<(), ServiceError> {
        Ok(jni::video_play(self.host_id, video_id)?)
    }
    fn pause(&self, video_id: u32) -> Result<(), ServiceError> {
        Ok(jni::video_pause(self.host_id, video_id)?)
    }
    fn stop(&self, video_id: u32) -> Result<(), ServiceError> {
        Ok(jni::video_stop(self.host_id, video_id)?)
    }
    fn seek(&self, video_id: u32, position: f64) -> Result<(), ServiceError> {
        Ok(jni::video_seek(self.host_id, video_id, position)?)
    }
    fn request_fullscreen(&self, video_id: u32, direction: i32) -> Result<(), ServiceError> {
        Ok(jni::video_request_fullscreen(
            self.host_id,
            video_id,
            direction,
        )?)
    }
    fn exit_fullscreen(&self, video_id: u32) -> Result<(), ServiceError> {
        Ok(jni::video_exit_fullscreen(self.host_id, video_id)?)
    }
    fn set_property(&self, video_id: u32, property_json: &str) -> Result<(), ServiceError> {
        Ok(jni::video_set_property(
            self.host_id,
            video_id,
            property_json,
        )?)
    }
    fn destroy(&self, video_id: u32) -> Result<(), ServiceError> {
        Ok(jni::video_destroy(self.host_id, video_id)?)
    }
}

// ==================== Location ====================

struct AndroidLocation {
    host_id: i32,
}

impl LocationService for AndroidLocation {
    fn get_location(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::get_location(self.host_id, options_json)?)
    }

    fn get_fuzzy_location(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::get_fuzzy_location(self.host_id, options_json)?)
    }
}

// ==================== Scan Code ====================

struct AndroidScanCode {
    host_id: i32,
}

impl ScanCodeService for AndroidScanCode {
    fn scan_code(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::scan_code(self.host_id, options_json)?)
    }
}

// ==================== Game Log ====================

struct AndroidGameLog {
    host_id: i32,
}

impl GameLogService for AndroidGameLog {
    fn report_log(&self, log_json: &str) -> Result<(), ServiceError> {
        Ok(jni::game_log_report(self.host_id, log_json)?)
    }
}

// ==================== Auth ====================

struct AndroidAuth {
    host_id: i32,
}

impl AuthService for AndroidAuth {
    fn login(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::auth_login(self.host_id, options_json)?)
    }

    fn check_session(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::auth_check_session(self.host_id, options_json)?)
    }

    fn get_user_info(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::auth_get_user_info(self.host_id, options_json)?)
    }

    fn get_phone_number(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::auth_get_phone_number(self.host_id, options_json)?)
    }
}

// ==================== Subpackage ====================

struct AndroidSubpackage {
    host_id: i32,
}

impl SubpackageService for AndroidSubpackage {
    fn download_subpackage(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::subpackage_download(self.host_id, options_json)?)
    }
}

// ==================== Share ====================

struct AndroidShare {
    host_id: i32,
}

impl ShareService for AndroidShare {
    fn share_app_message(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::share_app_message(self.host_id, options_json)?)
    }
}

// ==================== Navigate ====================

struct AndroidNavigate {
    host_id: i32,
}

impl NavigateService for AndroidNavigate {
    fn navigate_to_mini_program(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::navigate_to_mini_program(self.host_id, options_json)?)
    }

    fn open_customer_service_conversation(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::open_customer_service_conversation(
            self.host_id,
            options_json,
        )?)
    }
}

// ==================== Ads ====================

/// Forwards ad commands to the Java `AdHandler` the embedder installed.
///
/// Nothing is decided here: the reward verdict for incentivised video is
/// whatever the host's ad SDK reports on the `onAdEvent` channel. See
/// `shared/src/services/ad.rs` for why that has to be true.
struct AndroidAd {
    host_id: i32,
}

impl AdService for AndroidAd {
    fn create_ad(&self, request_json: &str) -> Result<(), ServiceError> {
        Ok(jni::ad_create(self.host_id, request_json)?)
    }

    fn load_ad(&self, request_json: &str) -> Result<(), ServiceError> {
        Ok(jni::ad_load(self.host_id, request_json)?)
    }

    fn show_ad(&self, request_json: &str) -> Result<(), ServiceError> {
        Ok(jni::ad_show(self.host_id, request_json)?)
    }

    fn hide_ad(&self, request_json: &str) -> Result<(), ServiceError> {
        Ok(jni::ad_hide(self.host_id, request_json)?)
    }

    fn update_ad_style(&self, request_json: &str) -> Result<(), ServiceError> {
        Ok(jni::ad_update_style(self.host_id, request_json)?)
    }

    fn destroy_ad(&self, request_json: &str) -> Result<(), ServiceError> {
        Ok(jni::ad_destroy(self.host_id, request_json)?)
    }
}

// ==================== Payment ====================

struct AndroidPayment {
    host_id: i32,
}

impl PaymentService for AndroidPayment {
    fn check_is_support_midas_payment(&self, options_json: &str) -> Result<String, ServiceError> {
        Ok(jni::check_is_support_midas_payment(
            self.host_id,
            options_json,
        )?)
    }

    fn request_midas_payment(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::request_midas_payment(self.host_id, options_json)?)
    }

    fn request_midas_payment_game_item(&self, options_json: &str) -> Result<(), ServiceError> {
        Ok(jni::request_midas_payment_game_item(
            self.host_id,
            options_json,
        )?)
    }
}
