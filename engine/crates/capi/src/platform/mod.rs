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
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
mod linux;
#[cfg(all(target_os = "linux", target_env = "ohos"))]
mod ohos;
#[cfg(not(any(
    target_os = "android",
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos")),
    all(target_os = "linux", target_env = "ohos")
)))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "android")]
pub(crate) use android::{PlatformTarget, build_target, rebuild_surface, supported_platform_kinds};
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
pub(crate) use linux::{PlatformTarget, build_target, rebuild_surface, supported_platform_kinds};
#[cfg(all(target_os = "linux", target_env = "ohos"))]
pub(crate) use ohos::{PlatformTarget, build_target, rebuild_surface, supported_platform_kinds};
#[cfg(not(any(
    target_os = "android",
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos")),
    all(target_os = "linux", target_env = "ohos")
)))]
pub(crate) use unsupported::{
    PlatformTarget, build_target, rebuild_surface, supported_platform_kinds,
};
#[cfg(target_os = "windows")]
pub(crate) use windows::{PlatformTarget, build_target, rebuild_surface, supported_platform_kinds};

#[cfg(test)]
mod contract_tests {
    use super::supported_platform_kinds;
    use migo_capi_abi::surface::MIGO_CAPI_IMPLEMENTED_PLATFORM_KINDS;

    /// Attaching a kind requires both halves: a platform module that can build
    /// the native objects, and a parser that will decode its payload.
    ///
    /// A kind present in only one half fails in a way that misdirects. Missing
    /// from the parser, attach returns UNSUPPORTED_PLATFORM even though the
    /// platform layer is right there, reading like the host passed the wrong
    /// kind. Missing from the platform module, the library loads, exports
    /// everything and advertises nothing -- which is how a Windows package
    /// shipped that could not attach a window.
    #[test]
    fn every_attachable_kind_is_also_parseable() {
        assert_eq!(
            supported_platform_kinds() & !MIGO_CAPI_IMPLEMENTED_PLATFORM_KINDS,
            0,
            "this build attaches a platform kind the ABI parser rejects"
        );
    }
}

/// Whether this build can attach the given `MIGO_PLATFORM_*` kind.
///
/// The single test both the attach path and the capability query go through. A
/// kind outside the bitmask's width is unsupported by definition rather than by
/// arithmetic accident.
pub(crate) fn kind_is_supported(platform_kind: u32) -> bool {
    if platform_kind >= u64::BITS {
        return false;
    }
    supported_platform_kinds() & (1u64 << platform_kind) != 0
}

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
