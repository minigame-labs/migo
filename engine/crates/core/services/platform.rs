use std::sync::Arc;

use deno_core::Extension;
use shared::config::InitOptions;

use super::DeviceServices;

/// Platform-specific services and extensions.
///
/// Each platform (Android, iOS, etc.) implements this trait to provide:
/// - Deno extensions with platform-specific ops
/// - Device services (clipboard, sensors, etc.)
/// - Error notification to the host app
pub trait PlatformServices: Send + Sync {
    /// Get platform-specific Deno extensions.
    fn extensions(&self, opts: &InitOptions) -> Vec<Extension>;

    /// Create device services for a specific host session.
    ///
    /// # Arguments
    /// * `host_id` - The session/host identifier for this runtime instance
    ///
    /// Returns None if device services are not available on this platform.
    fn create_device_services(&self, _host_id: i32) -> Option<Arc<dyn DeviceServices>> {
        None
    }

    /// Notify the host application that the mini program is exiting.
    ///
    /// This is called when the JS side calls `exitMiniProgram`.
    fn notify_exit(&self, _host_id: i32) {
        // Default: no-op.
    }

    /// Notify the host application about a fatal engine error.
    ///
    /// This is called when:
    /// - The host thread panics (Rust panic in catch_unwind)
    /// - The ANR watchdog terminates the isolate
    /// - V8 heap limit / execution timeout is triggered
    ///
    /// On Android, this calls `NativeExports.onError(hostId, code, msg, detail)`
    /// via JNI.  The watchdog thread also uses this to report ANR events.
    ///
    /// Default implementation is a no-op (for platforms that haven't implemented
    /// error notification yet).
    ///
    /// # Arguments
    /// * `host_id` - The host session ID
    /// * `error_code` - The `ErrorCode` as u16
    /// * `message` - Human-readable error message
    /// * `detail` - Additional debugging details
    fn notify_error(&self, _host_id: i32, _error_code: u16, _message: &str, _detail: &str) {
        // Default: no-op. Override on platforms with JNI / native callbacks.
    }
}
