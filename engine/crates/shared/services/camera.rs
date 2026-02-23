//! Camera service trait for platform camera access.

/// Camera service for photo capture, video recording, and frame streaming.
///
/// Platforms implement this trait to provide camera capabilities.
/// Commands that return data use JSON strings for flexibility.
/// Fire-and-forget commands return `Result<(), String>`.
///
/// Event delivery (frame data, stop, auth cancel) is handled by the platform
/// pushing events back to the JS runtime via `_internalOnCameraEvent` and
/// `_internalOnCameraFrameData`.
pub trait CameraService: Send + Sync {
    /// Create a camera instance with the given configuration (JSON).
    ///
    /// JSON fields:
    /// - `cameraId`: u32 - JS-assigned camera instance ID
    /// - `pos`: "back" | "front" (default "back")
    /// - `flash`: "auto" | "on" | "off" | "torch" (default "auto")
    /// - `size`: "medium" | "small" | "large" (default "medium")
    ///
    /// Returns JSON: `{"cameraId": <id>}` on success.
    fn create(&self, _options_json: &str) -> Result<String, String> {
        Err("createCamera:fail not supported".to_string())
    }

    /// Destroy a camera instance and release all resources.
    fn destroy(&self, _camera_id: u32) -> Result<(), String> {
        Err("camera.destroy:fail not supported".to_string())
    }

    /// Take a photo.
    ///
    /// JSON fields:
    /// - `cameraId`: u32
    /// - `quality`: "high" | "normal" | "low" (default "normal")
    ///
    /// Returns JSON: `{"tempImagePath": "<path>"}` on success.
    fn take_photo(&self, _options_json: &str) -> Result<String, String> {
        Err("camera.takePhoto:fail not supported".to_string())
    }

    /// Start video recording.
    ///
    /// JSON fields:
    /// - `cameraId`: u32
    /// - `timeout`: number (max recording duration in seconds, optional)
    ///
    /// Returns JSON: `{}` on success (recording started).
    /// When timeout is reached, platform fires a "timeoutCallback" event.
    fn start_record(&self, _options_json: &str) -> Result<String, String> {
        Err("camera.startRecord:fail not supported".to_string())
    }

    /// Stop video recording.
    ///
    /// JSON fields:
    /// - `cameraId`: u32
    /// - `compressed`: bool (default false)
    ///
    /// Returns JSON: `{"tempThumbPath": "<path>", "tempVideoPath": "<path>"}` on success.
    fn stop_record(&self, _options_json: &str) -> Result<String, String> {
        Err("camera.stopRecord:fail not supported".to_string())
    }

    /// Set camera zoom level.
    ///
    /// JSON fields:
    /// - `cameraId`: u32
    /// - `zoom`: number (zoom factor)
    ///
    /// Returns JSON: `{"zoom": <actual_zoom>}` on success.
    fn set_zoom(&self, _options_json: &str) -> Result<String, String> {
        Err("camera.setZoom:fail not supported".to_string())
    }

    /// Start listening for camera frame changes (high-frequency streaming).
    ///
    /// After calling this, the platform should continuously push frame data via
    /// `_internalOnCameraFrameData(cameraId, arrayBuffer, width, height)`.
    fn listen_frame_change(&self, _camera_id: u32) -> Result<(), String> {
        Err("camera.listenFrameChange:fail not supported".to_string())
    }

    /// Stop listening for camera frame changes.
    fn close_frame_change(&self, _camera_id: u32) -> Result<(), String> {
        Err("camera.closeFrameChange:fail not supported".to_string())
    }
}
