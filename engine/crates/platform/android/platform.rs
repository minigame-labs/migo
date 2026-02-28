use std::sync::Arc;

use core::services::DeviceServices;
use core::PlatformServices;
use deno_core::Extension;
use shared::config::InitOptions;
use tracing::error;

use crate::android::extensions::android_extensions;
use crate::android::jni;
use crate::android::services::AndroidDeviceServices;

pub struct AndroidPlatform;

impl AndroidPlatform {
    pub fn new() -> Self {
        Self
    }
}

impl PlatformServices for AndroidPlatform {
    fn extensions(&self, opts: &InitOptions) -> Vec<Extension> {
        android_extensions(opts)
    }

    fn create_device_services(&self, host_id: i32) -> Option<Arc<dyn DeviceServices>> {
        Some(Arc::new(AndroidDeviceServices::new(host_id)))
    }

    fn notify_exit(&self, host_id: i32) {
        if let Err(e) = jni::notify_exit(host_id) {
            error!("[Host {}] Failed to notify Java of exit: {}", host_id, e);
        }
    }

    fn notify_error(&self, host_id: i32, error_code: u16, message: &str, detail: &str) {
        if let Err(e) = jni::notify_error(host_id, error_code, message, detail) {
            error!(
                "[Host {}] Failed to notify Java of error (code={}): {}",
                host_id, error_code, e
            );
        }
    }
}
