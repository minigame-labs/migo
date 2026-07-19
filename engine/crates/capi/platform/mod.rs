//! Where the ABI meets a platform.
//!
//! Every platform detail the entry points would otherwise name lives behind
//! this module: which surface type wraps the host's window, which graphics
//! platform drives it, and which `PlatformServices` implementation supplies
//! device services and the frame clock. The entry points stay free of `cfg`,
//! so adding a platform means adding a file here rather than editing the
//! middle of `surface.rs`.

#[cfg(target_os = "android")]
mod android;
#[cfg(not(target_os = "android"))]
mod desktop;

#[cfg(target_os = "android")]
pub(crate) use android::{build_target, rebuild_surface, PlatformTarget};
#[cfg(not(target_os = "android"))]
pub(crate) use desktop::{build_target, rebuild_surface, PlatformTarget};

/// The platform's own `PlatformServices`, which `CapiHostKit` composes so it
/// only has to override the notification capability.
#[cfg(target_os = "android")]
pub(crate) type InnerPlatform = platform::android::platform::AndroidPlatform;
#[cfg(not(target_os = "android"))]
pub(crate) type InnerPlatform = platform::desktop::platform::DesktopPlatform;
