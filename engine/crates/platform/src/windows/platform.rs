use migo_core::services::{
    CommerceServices, ConnectivityServices, DeviceServices, KeyboardService, MediaServices,
    SensorServices, SystemUtilServices,
};
use migo_core::{DeviceServiceProvider, FrameClock, HostNotifier};
use std::sync::Arc;

/// Windows platform capabilities.
///
/// Shaped like `LinuxPlatform` rather than `AndroidPlatform` because the
/// situation is the same: the engine layer has no device services of its own
/// here, so anything content asks for has to come from the embedding host.
pub struct WindowsPlatform {
    /// Supplied by an embedding host that services the keyboard itself.
    host_keyboard: Option<Arc<dyn KeyboardService>>,
}

impl WindowsPlatform {
    pub fn new() -> Self {
        Self::with_host_keyboard(None)
    }

    /// Build a platform that offers a host-supplied keyboard.
    ///
    /// Offered exactly when the host supplied one — the same rule
    /// `FrameClock::uses_external_vsync` follows: report what the host actually
    /// installed, not what the platform would prefer.
    pub fn with_host_keyboard(host_keyboard: Option<Arc<dyn KeyboardService>>) -> Self {
        Self { host_keyboard }
    }
}

impl Default for WindowsPlatform {
    fn default() -> Self {
        Self::new()
    }
}

/// Windows' bundle exists only to carry a host-supplied capability.
///
/// Every other accessor keeps its default, which is not an omission: there is
/// nothing behind them to forward to yet, and a forwarding layer that answered
/// `None` for a service the platform gained later would drop it silently.
struct WindowsDeviceServices {
    host_keyboard: Arc<dyn KeyboardService>,
}

impl SensorServices for WindowsDeviceServices {}
impl MediaServices for WindowsDeviceServices {}
impl ConnectivityServices for WindowsDeviceServices {}
impl CommerceServices for WindowsDeviceServices {}
impl SystemUtilServices for WindowsDeviceServices {
    fn keyboard(&self) -> Option<Arc<dyn KeyboardService>> {
        Some(Arc::clone(&self.host_keyboard))
    }
}

impl DeviceServiceProvider for WindowsPlatform {
    fn create_device_services(&self, _host_id: i32) -> Option<Arc<dyn DeviceServices>> {
        // Without a host keyboard there is still nothing to offer, so returning
        // a bundle unconditionally would only change what callers see.
        let host_keyboard = self.host_keyboard.as_ref()?;
        Some(Arc::new(WindowsDeviceServices {
            host_keyboard: Arc::clone(host_keyboard),
        }))
    }
}

impl FrameClock for WindowsPlatform {}

impl HostNotifier for WindowsPlatform {}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::protocol::error::ServiceError;

    struct HostKeyboard;
    impl KeyboardService for HostKeyboard {
        fn show(&self, _options_json: &str) -> Result<(), ServiceError> {
            Ok(())
        }
    }

    /// No DWM/composition vsync channel is wired yet, so the engine must keep
    /// driving frames itself rather than wait for a signal nobody sends.
    #[test]
    fn windows_keeps_the_software_frame_ticker_until_a_host_supplies_vsync() {
        assert!(!WindowsPlatform::new().uses_external_vsync());
    }

    #[test]
    fn without_a_host_keyboard_there_are_still_no_device_services() {
        assert!(WindowsPlatform::new().create_device_services(1).is_none());
    }

    #[test]
    fn a_host_supplied_keyboard_becomes_the_bundles_keyboard() {
        let platform = WindowsPlatform::with_host_keyboard(Some(Arc::new(HostKeyboard)));
        let services = platform
            .create_device_services(1)
            .expect("a supplied keyboard must produce a bundle");
        assert!(services.clipboard().is_none());
        assert!(
            services
                .keyboard()
                .expect("the host's keyboard must be offered")
                .show("{}")
                .is_ok(),
            "the bundle must hand back the host's own keyboard, not a default"
        );
    }
}
