use std::sync::Arc;
use core::PlatformServices;
use core::services::DeviceServices;
use shared::config::InitOptions;
use deno_core::Extension;

pub struct DesktopPlatform;

impl DesktopPlatform {
    pub fn new() -> Self {
        Self
    }
}

impl PlatformServices for DesktopPlatform {
    fn extensions(&self, _opts: &InitOptions) -> Vec<Extension> {
        vec![]
    }

    fn create_device_services(&self, _host_id: i32) -> Option<Arc<dyn DeviceServices>> {
        None
    }
}
