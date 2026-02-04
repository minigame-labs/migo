use std::sync::Arc;

use deno_core::Extension;
use shared::config::InitOptions;

use super::DeviceServices;

/// Platform-specific services and extensions.
///
/// Each platform (Android, iOS, etc.) implements this trait to provide:
/// - Deno extensions with platform-specific ops
/// - Device services (clipboard, sensors, etc.)
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
}
