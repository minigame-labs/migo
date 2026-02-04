use std::sync::Arc;

use core::services::DeviceServices;
use core::PlatformServices;
use deno_core::Extension;
use shared::config::InitOptions;

use crate::android::extensions::android_extensions;
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
}
