//! Android device service implementations.
//!
//! Implements `core::services::DeviceServices` traits using JNI calls.

use std::sync::Arc;

use core::services::{
    AccelerometerService, AudioPlatformService, BatteryService, CameraService, ClipboardService,
    CompassService, DeviceMotionService, DeviceServices, GyroscopeService, NetworkService,
    RecorderService, ScreenService, VibrationService,
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
