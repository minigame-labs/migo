//! Platform services for a C host.
//!
//! The platform's own `PlatformServices` already provides device info and the
//! frame clock; only the notification capability differs, because engine events have to reach C
//! through the host's dispatcher instead of being dropped. Composing rather
//! than reimplementing keeps that difference to exactly one trait — which is
//! what the step-6 capability split was for.

use core::{DeviceServiceProvider, FrameClock, HostNotifier};
use crate::platform::InnerPlatform;

use crate::{abi::MIGO_ERROR_INTERNAL, callbacks::Notifier};

pub struct CapiHostKit {
    inner: InnerPlatform,
    notifier: Option<Notifier>,
}

impl CapiHostKit {
    pub fn new(notifier: Option<Notifier>) -> Self {
        Self {
            inner: InnerPlatform::new(),
            notifier,
        }
    }
}

impl DeviceServiceProvider for CapiHostKit {
    fn create_device_services(
        &self,
        host_id: i32,
    ) -> Option<std::sync::Arc<dyn shared::services::DeviceServices>> {
        self.inner.create_device_services(host_id)
    }
}

impl FrameClock for CapiHostKit {
    /// Whether the host offered to drive frames, not what the platform would
    /// prefer.
    ///
    /// Android's own `PlatformServices` answers true and drives frames by
    /// calling into the Java SDK over JNI, which a C host does not have. The
    /// honest answer for a C host is whatever it actually supplied: a host that
    /// installed `on_request_frame` paces frames itself -- with AChoreographer,
    /// a compositor frame callback, whatever its platform offers -- and one
    /// that did not is paced by the engine.
    ///
    /// Answering the platform's preference instead left the engine waiting for
    /// a vsync nobody would deliver: on device the window attached, content
    /// loaded and reported ready, and not one frame was drawn.
    fn uses_external_vsync(&self) -> bool {
        self.notifier
            .as_ref()
            .is_some_and(|notifier| notifier.drives_frames())
    }

    fn request_vsync(&self, _host_id: i32) {
        if let Some(notifier) = &self.notifier {
            notifier.request_frame();
        }
    }
}

impl HostNotifier for CapiHostKit {
    fn notify_game_ready(&self, host_id: i32) {
        self.inner.notify_game_ready(host_id);
        if let Some(notifier) = &self.notifier {
            notifier.ready();
        }
    }

    fn notify_exit(&self, host_id: i32) {
        self.inner.notify_exit(host_id);
        if let Some(notifier) = &self.notifier {
            notifier.exit_requested();
        }
    }

    fn notify_error(&self, host_id: i32, code: u16, msg: &str, detail: &str) {
        self.inner.notify_error(host_id, code, msg, detail);
        if let Some(notifier) = &self.notifier {
            // The engine's own code space is not the ABI's, so report a stable
            // ABI code and carry the engine's numbering in the message where a
            // host can still log it.
            let text = if detail.is_empty() {
                format!("{msg} (engine code {code})")
            } else {
                format!("{msg}: {detail} (engine code {code})")
            };
            notifier.error(MIGO_ERROR_INTERNAL, text);
        }
    }
}
