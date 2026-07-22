use migo_core::services::{
    CommerceServices, ConnectivityServices, DeviceServices, KeyboardService, MediaServices,
    SensorServices, SystemUtilServices,
};
use migo_core::{DeviceServiceProvider, FrameClock, HostNotifier};
use std::sync::Arc;

pub struct LinuxPlatform {
    /// Supplied by an embedding host that services the keyboard itself.
    ///
    /// The Linux engine layer has no keyboard of its own, so this is the only way content's
    /// `wx.showKeyboard` reaches anything at all.
    host_keyboard: Option<Arc<dyn KeyboardService>>,
}

impl LinuxPlatform {
    pub fn new() -> Self {
        Self::with_host_keyboard(None)
    }

    /// Build a platform that offers a host-supplied keyboard.
    ///
    /// The capability is offered exactly when the host supplied one, which is
    /// the rule `FrameClock::uses_external_vsync` already follows: report what
    /// the host actually installed, not what the platform would prefer.
    pub fn with_host_keyboard(host_keyboard: Option<Arc<dyn KeyboardService>>) -> Self {
        Self { host_keyboard }
    }
}

impl Default for LinuxPlatform {
    fn default() -> Self {
        Self::new()
    }
}

/// Linux's bundle exists only to carry a host-supplied capability.
///
/// Every other accessor keeps its default. That is not an omission: Linux has
/// nothing behind them to forward to, and a forwarding layer that answered
/// `None` for a service the platform gained later would drop it silently.
struct LinuxDeviceServices {
    host_keyboard: Arc<dyn KeyboardService>,
}

impl SensorServices for LinuxDeviceServices {}
impl MediaServices for LinuxDeviceServices {}
impl ConnectivityServices for LinuxDeviceServices {}
impl CommerceServices for LinuxDeviceServices {}
impl SystemUtilServices for LinuxDeviceServices {
    fn keyboard(&self) -> Option<Arc<dyn KeyboardService>> {
        Some(Arc::clone(&self.host_keyboard))
    }
}

impl DeviceServiceProvider for LinuxPlatform {
    fn create_device_services(&self, _host_id: i32) -> Option<Arc<dyn DeviceServices>> {
        // Returning a bundle unconditionally would change what every existing
        // Linux caller sees; without a host keyboard there is still nothing
        // to offer.
        let host_keyboard = self.host_keyboard.as_ref()?;
        Some(Arc::new(LinuxDeviceServices {
            host_keyboard: Arc::clone(host_keyboard),
        }))
    }
}

impl FrameClock for LinuxPlatform {}

impl HostNotifier for LinuxPlatform {}

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

    #[test]
    fn linux_player_keeps_software_frame_ticker() {
        assert!(!LinuxPlatform::new().uses_external_vsync());
    }

    /// The Linux engine layer has no device services of its own, and that must not change for
    /// a host that supplies nothing.
    #[test]
    fn without_a_host_keyboard_there_are_still_no_device_services() {
        assert!(LinuxPlatform::new().create_device_services(1).is_none());
    }

    /// A host that supplies a keyboard gets a bundle that offers it -- and
    /// offers nothing else, because the Linux engine layer has nothing else to offer.
    #[test]
    fn a_host_supplied_keyboard_becomes_the_bundles_keyboard() {
        let platform = LinuxPlatform::with_host_keyboard(Some(Arc::new(HostKeyboard)));
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
