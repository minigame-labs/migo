use core::services::DeviceServices;
use core::{DeviceServiceProvider, FrameClock, HostNotifier};
use std::sync::Arc;

pub struct DesktopPlatform;

impl DesktopPlatform {
    pub fn new() -> Self {
        Self
    }
}

impl DeviceServiceProvider for DesktopPlatform {
    fn create_device_services(&self, _host_id: i32) -> Option<Arc<dyn DeviceServices>> {
        None
    }
}

impl FrameClock for DesktopPlatform {}

impl HostNotifier for DesktopPlatform {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_keeps_software_frame_ticker() {
        assert!(!DesktopPlatform::new().uses_external_vsync());
    }
}
