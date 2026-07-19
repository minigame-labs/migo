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

/// Send the opt-in developer log to wherever this platform's diagnostics go.
///
/// A subscriber that writes to standard output is only a diagnostic channel on
/// a platform that shows standard output. Android discards it, so a C host
/// there saw nothing at all from the engine -- the one channel the header
/// promises was silently empty on the platform where a host has fewest
/// alternatives.
#[cfg(target_os = "android")]
pub(crate) fn install_dev_logging(level: shared::config::LogLevel) {
    platform::android::install_logcat_diagnostics(level);
}

#[cfg(not(target_os = "android"))]
pub(crate) fn install_dev_logging(level: shared::config::LogLevel) {
    use shared::config::LogLevel;
    let level = match level {
        LogLevel::Trace => tracing::Level::TRACE,
        LogLevel::Debug => tracing::Level::DEBUG,
        LogLevel::Info => tracing::Level::INFO,
        LogLevel::Warn => tracing::Level::WARN,
        LogLevel::Error | LogLevel::Off => tracing::Level::ERROR,
    };
    let _ = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(true)
        .try_init();
}
