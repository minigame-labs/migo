//! Android device service implementations.
//!
//! Implements `core::services::DeviceServices` traits using JNI calls.

use std::sync::Arc;

use core::services::{
    AccelerometerService, AudioPlatformService, BatteryService, BluetoothService, CameraService,
    ClipboardService, CodecService, CompassService, DeviceMotionService, DeviceServices,
    FileService, GyroscopeService, ImageApiService, InteractionService, KeyboardService,
    LocationService, NetworkService, RecorderService, ScanCodeService, ScreenService,
    SystemInfoService, VibrationService,
};

use crate::android::jni;

/// Android device services aggregator.
pub struct AndroidDeviceServices {
    host_id: i32,
}

impl AndroidDeviceServices {
    pub fn new(host_id: i32) -> Self {
        Self { host_id }
    }
}

impl DeviceServices for AndroidDeviceServices {
    fn clipboard(&self) -> Option<Arc<dyn ClipboardService>> {
        Some(Arc::new(AndroidClipboard { host_id: self.host_id }))
    }

    fn battery(&self) -> Option<Arc<dyn BatteryService>> {
        Some(Arc::new(AndroidBattery))
    }

    fn vibration(&self) -> Option<Arc<dyn VibrationService>> {
        Some(Arc::new(AndroidVibration))
    }

    fn screen(&self) -> Option<Arc<dyn ScreenService>> {
        Some(Arc::new(AndroidScreen { host_id: self.host_id }))
    }

    fn device_motion(&self) -> Option<Arc<dyn DeviceMotionService>> {
        Some(Arc::new(AndroidDeviceMotion { host_id: self.host_id }))
    }

    fn gyroscope(&self) -> Option<Arc<dyn GyroscopeService>> {
        Some(Arc::new(AndroidGyroscope { host_id: self.host_id }))
    }

    fn compass(&self) -> Option<Arc<dyn CompassService>> {
        Some(Arc::new(AndroidCompass { host_id: self.host_id }))
    }

    fn accelerometer(&self) -> Option<Arc<dyn AccelerometerService>> {
        Some(Arc::new(AndroidAccelerometer { host_id: self.host_id }))
    }

    fn network(&self) -> Option<Arc<dyn NetworkService>> {
        Some(Arc::new(AndroidNetwork { host_id: self.host_id }))
    }

    fn audio_platform(&self) -> Option<Arc<dyn AudioPlatformService>> {
        Some(Arc::new(AndroidAudioPlatform { host_id: self.host_id }))
    }

    fn recorder(&self) -> Option<Arc<dyn RecorderService>> {
        Some(Arc::new(AndroidRecorder { host_id: self.host_id }))
    }

    fn camera(&self) -> Option<Arc<dyn CameraService>> {
        Some(Arc::new(AndroidCamera { host_id: self.host_id }))
    }

    fn interaction(&self) -> Option<Arc<dyn InteractionService>> {
        Some(Arc::new(AndroidInteraction { host_id: self.host_id }))
    }

    fn system_info(&self) -> Option<Arc<dyn SystemInfoService>> {
        Some(Arc::new(AndroidSystemInfo { host_id: self.host_id }))
    }

    fn codec(&self) -> Option<Arc<dyn CodecService>> {
        Some(Arc::new(AndroidCodec))
    }

    fn file(&self) -> Option<Arc<dyn FileService>> {
        Some(Arc::new(AndroidFile))
    }

    fn bluetooth(&self) -> Option<Arc<dyn BluetoothService>> {
        Some(Arc::new(AndroidBluetooth { host_id: self.host_id }))
    }

    fn keyboard(&self) -> Option<Arc<dyn KeyboardService>> {
        Some(Arc::new(AndroidKeyboard { host_id: self.host_id }))
    }

    fn image_api(&self) -> Option<Arc<dyn ImageApiService>> {
        Some(Arc::new(AndroidImageApi { host_id: self.host_id }))
    }

    fn location(&self) -> Option<Arc<dyn LocationService>> {
        Some(Arc::new(AndroidLocation { host_id: self.host_id }))
    }

    fn scan_code(&self) -> Option<Arc<dyn ScanCodeService>> {
        Some(Arc::new(AndroidScanCode { host_id: self.host_id }))
    }
}

// ==================== Clipboard ====================

struct AndroidClipboard {
    host_id: i32,
}

impl ClipboardService for AndroidClipboard {
    fn set_data(&self, data: &str) -> Result<(), String> {
        jni::set_clipboard_data(self.host_id, data)
    }

    fn get_data(&self) -> Result<String, String> {
        jni::get_clipboard_data(self.host_id)
    }
}

// ==================== Battery ====================

struct AndroidBattery;

impl BatteryService for AndroidBattery {
    fn get_info_json(&self) -> Result<String, String> {
        jni::get_battery_info_json()
    }
}

// ==================== Vibration ====================

struct AndroidVibration;

impl VibrationService for AndroidVibration {
    fn vibrate_short(&self, type_: &str) -> Result<(), String> {
        jni::vibrate_short(type_).map(|_| ())
    }

    fn vibrate_long(&self) -> Result<(), String> {
        jni::vibrate_long().map(|_| ())
    }
}

// ==================== Screen ====================

struct AndroidScreen {
    host_id: i32,
}

impl ScreenService for AndroidScreen {
    fn get_brightness(&self) -> Result<f32, String> {
        jni::get_screen_brightness(self.host_id)
    }

    fn set_brightness(&self, value: f32) -> Result<(), String> {
        jni::set_screen_brightness(self.host_id, value).map(|_| ())
    }

    fn set_keep_screen_on(&self, keep_on: bool) -> Result<(), String> {
        jni::set_keep_screen_on(self.host_id, keep_on).map(|_| ())
    }

    fn set_orientation(&self, value: &str) -> Result<(), String> {
        jni::set_device_orientation(self.host_id, value).map(|_| ())
    }

    fn start_capture_screen(&self) -> Result<(), String> {
        jni::start_capture_screen(self.host_id)
    }

    fn stop_capture_screen(&self) -> Result<(), String> {
        jni::stop_capture_screen(self.host_id)
    }

    fn set_enable_debug(&self, enabled: bool) -> Result<(), String> {
        jni::set_enable_debug(self.host_id, enabled).map(|_| ())
    }
}

// ==================== Device Motion ====================

struct AndroidDeviceMotion {
    host_id: i32,
}

impl DeviceMotionService for AndroidDeviceMotion {
    fn start(&self, interval: &str) -> Result<(), String> {
        jni::start_device_motion(self.host_id, interval)
    }

    fn stop(&self) -> Result<(), String> {
        jni::stop_device_motion(self.host_id)
    }
}

// ==================== Gyroscope ====================

struct AndroidGyroscope {
    host_id: i32,
}

impl GyroscopeService for AndroidGyroscope {
    fn start(&self, interval: &str) -> Result<(), String> {
        jni::start_gyroscope(self.host_id, interval)
    }

    fn stop(&self) -> Result<(), String> {
        jni::stop_gyroscope(self.host_id)
    }
}

// ==================== Compass ====================

struct AndroidCompass {
    host_id: i32,
}

impl CompassService for AndroidCompass {
    fn start(&self) -> Result<(), String> {
        jni::start_compass(self.host_id)
    }

    fn stop(&self) -> Result<(), String> {
        jni::stop_compass(self.host_id)
    }
}

// ==================== Accelerometer ====================

struct AndroidAccelerometer {
    host_id: i32,
}

impl AccelerometerService for AndroidAccelerometer {
    fn start(&self, interval: &str) -> Result<(), String> {
        jni::start_accelerometer(self.host_id, interval)
    }

    fn stop(&self) -> Result<(), String> {
        jni::stop_accelerometer(self.host_id)
    }
}

// ==================== Network ====================

struct AndroidNetwork {
    host_id: i32,
}

impl NetworkService for AndroidNetwork {
    fn start_monitoring(&self) -> Result<(), String> {
        jni::start_network_monitoring(self.host_id)
    }

    fn stop_monitoring(&self) -> Result<(), String> {
        jni::stop_network_monitoring(self.host_id)
    }

    fn get_network_type_json(&self) -> Result<String, String> {
        jni::get_network_type_json(self.host_id)
    }

    fn get_local_ip_json(&self) -> Result<String, String> {
        jni::get_local_ip_address_json()
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
    ) -> Result<(), String> {
        jni::set_inner_audio_option(self.host_id, mix_with_other, obey_mute_switch, speaker_on)
    }

    fn get_available_audio_sources(&self) -> Result<Vec<String>, String> {
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
    fn start(&self, options_json: &str) -> Result<(), String> {
        jni::recorder_start(self.host_id, options_json)
    }

    fn pause(&self) -> Result<(), String> {
        jni::recorder_pause(self.host_id)
    }

    fn resume(&self) -> Result<(), String> {
        jni::recorder_resume(self.host_id)
    }

    fn stop(&self) -> Result<(), String> {
        jni::recorder_stop(self.host_id)
    }
}

// ==================== Camera ====================

struct AndroidCamera {
    host_id: i32,
}

impl CameraService for AndroidCamera {
    fn create(&self, options_json: &str) -> Result<String, String> {
        jni::camera_create(self.host_id, options_json)
    }

    fn destroy(&self, camera_id: u32) -> Result<(), String> {
        jni::camera_destroy(self.host_id, camera_id)
    }

    fn take_photo(&self, options_json: &str) -> Result<String, String> {
        jni::camera_take_photo(self.host_id, options_json)
    }

    fn start_record(&self, options_json: &str) -> Result<String, String> {
        jni::camera_start_record(self.host_id, options_json)
    }

    fn stop_record(&self, options_json: &str) -> Result<String, String> {
        jni::camera_stop_record(self.host_id, options_json)
    }

    fn set_zoom(&self, options_json: &str) -> Result<String, String> {
        jni::camera_set_zoom(self.host_id, options_json)
    }

    fn listen_frame_change(&self, camera_id: u32) -> Result<(), String> {
        jni::camera_listen_frame_change(self.host_id, camera_id)
    }

    fn close_frame_change(&self, camera_id: u32) -> Result<(), String> {
        jni::camera_close_frame_change(self.host_id, camera_id)
    }
}

// ==================== UI Interaction ====================

struct AndroidInteraction {
    host_id: i32,
}

impl InteractionService for AndroidInteraction {
    fn show_toast(&self, json: &str) -> Result<(), String> {
        jni::show_toast(self.host_id, json)
    }

    fn hide_toast(&self) -> Result<(), String> {
        jni::hide_toast(self.host_id)
    }

    fn show_modal(&self, json: &str) -> Result<(), String> {
        jni::show_modal(self.host_id, json)
    }

    fn show_loading(&self, json: &str) -> Result<(), String> {
        jni::show_loading(self.host_id, json)
    }

    fn hide_loading(&self) -> Result<(), String> {
        jni::hide_loading(self.host_id)
    }

    fn show_action_sheet(&self, json: &str) -> Result<(), String> {
        jni::show_action_sheet(self.host_id, json)
    }
}

// ==================== System Info ====================

struct AndroidSystemInfo {
    host_id: i32,
}

impl SystemInfoService for AndroidSystemInfo {
    fn open_bluetooth_settings(&self) -> Result<(), String> {
        jni::open_bluetooth_settings(self.host_id)
    }

    fn open_app_authorize_setting(&self) -> Result<(), String> {
        jni::open_app_authorize_setting(self.host_id)
    }

    fn get_window_info_json(&self) -> Result<String, String> {
        let info = jni::get_window_info(self.host_id)?;
        deno_core::serde_json::to_string(&info).map_err(|e| format!("getWindowInfo:fail {}", e))
    }

    fn get_system_settings_json(&self) -> Result<String, String> {
        let settings = jni::get_system_settings()?;
        deno_core::serde_json::to_string(&settings).map_err(|e| format!("getSystemSetting:fail {}", e))
    }

    fn get_device_info_json(&self) -> Result<String, String> {
        jni::get_device_info_json()
    }

    fn get_app_authorization_setting_json(&self) -> Result<String, String> {
        jni::get_app_authorization_setting_json()
    }
}

// ==================== Bluetooth ====================

struct AndroidBluetooth {
    host_id: i32,
}

impl BluetoothService for AndroidBluetooth {
    fn open_adapter(&self, options_json: &str) -> Result<(), String> {
        jni::bluetooth_open_adapter(self.host_id, options_json)
    }

    fn close_adapter(&self) -> Result<(), String> {
        jni::bluetooth_close_adapter(self.host_id)
    }

    fn get_adapter_state(&self) -> Result<String, String> {
        jni::bluetooth_get_adapter_state(self.host_id)
    }

    fn start_devices_discovery(&self, options_json: &str) -> Result<(), String> {
        jni::bluetooth_start_devices_discovery(self.host_id, options_json)
    }

    fn stop_devices_discovery(&self) -> Result<(), String> {
        jni::bluetooth_stop_devices_discovery(self.host_id)
    }

    fn get_devices(&self) -> Result<String, String> {
        jni::bluetooth_get_devices(self.host_id)
    }

    fn get_connected_devices(&self, options_json: &str) -> Result<String, String> {
        jni::bluetooth_get_connected_devices(self.host_id, options_json)
    }

    fn make_pair(&self, options_json: &str) -> Result<(), String> {
        jni::bluetooth_make_pair(self.host_id, options_json)
    }

    fn is_device_paired(&self, options_json: &str) -> Result<(), String> {
        jni::bluetooth_is_device_paired(self.host_id, options_json)
    }

    fn start_beacon_discovery(&self, options_json: &str) -> Result<(), String> {
        jni::bluetooth_start_beacon_discovery(self.host_id, options_json)
    }

    fn stop_beacon_discovery(&self) -> Result<(), String> {
        jni::bluetooth_stop_beacon_discovery(self.host_id)
    }

    fn get_beacons(&self) -> Result<String, String> {
        jni::bluetooth_get_beacons(self.host_id)
    }
}

// ==================== Keyboard ====================

struct AndroidKeyboard {
    host_id: i32,
}

impl KeyboardService for AndroidKeyboard {
    fn show(&self, options_json: &str) -> Result<(), String> {
        jni::keyboard_show(self.host_id, options_json)
    }

    fn hide(&self) -> Result<(), String> {
        jni::keyboard_hide(self.host_id)
    }

    fn update(&self, value: &str) -> Result<(), String> {
        jni::keyboard_update(self.host_id, value)
    }
}

// ==================== Codec (GBK) ====================

struct AndroidCodec;

impl CodecService for AndroidCodec {
    fn encode_gbk(&self, data: &str) -> Result<Vec<u8>, String> {
        jni::outbound::encode_gbk(data)
    }

    fn decode_gbk(&self, data: &[u8]) -> Result<String, String> {
        jni::outbound::decode_gbk(data)
    }
}

// ==================== File (Unzip) ====================

struct AndroidFile;

impl FileService for AndroidFile {
    fn unzip(&self, zip_path: &str, dest_dir: &str) -> Result<usize, String> {
        jni::outbound::unzip_file(zip_path, dest_dir)
    }
}

// ==================== Image API ====================

struct AndroidImageApi {
    host_id: i32,
}

impl ImageApiService for AndroidImageApi {
    fn save_image_to_photos_album(&self, options_json: &str) -> Result<(), String> {
        jni::image_save_to_photos_album(self.host_id, options_json)
    }

    fn preview_media(&self, options_json: &str) -> Result<(), String> {
        jni::image_preview_media(self.host_id, options_json)
    }

    fn preview_image(&self, options_json: &str) -> Result<(), String> {
        jni::image_preview_image(self.host_id, options_json)
    }

    fn compress_image(&self, options_json: &str) -> Result<(), String> {
        jni::image_compress(self.host_id, options_json)
    }

    fn choose_message_file(&self, options_json: &str) -> Result<(), String> {
        jni::image_choose_message_file(self.host_id, options_json)
    }

    fn choose_image(&self, options_json: &str) -> Result<(), String> {
        jni::image_choose_image(self.host_id, options_json)
    }
}

// ==================== Location ====================

struct AndroidLocation {
    host_id: i32,
}

impl LocationService for AndroidLocation {
    fn get_location(&self, options_json: &str) -> Result<(), String> {
        jni::get_location(self.host_id, options_json)
    }

    fn get_fuzzy_location(&self, options_json: &str) -> Result<(), String> {
        jni::get_fuzzy_location(self.host_id, options_json)
    }
}

// ==================== Scan Code ====================

struct AndroidScanCode {
    host_id: i32,
}

impl ScanCodeService for AndroidScanCode {
    fn scan_code(&self, options_json: &str) -> Result<(), String> {
        jni::scan_code(self.host_id, options_json)
    }
}
