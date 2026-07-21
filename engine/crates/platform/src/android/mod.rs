pub mod jni;
pub(crate) mod logging;

/// Send engine diagnostics to logcat at `level`.
///
/// The JNI path reaches this through its own session setup; a C host has no
/// JNI, so this is how an embedder that never touches Java asks for the same
/// channel. Kept as one function rather than a module re-export so what is
/// public stays the capability, not the implementation.
pub fn install_logcat_diagnostics(level: shared::config::LogLevel) {
    logging::init_logging();
    logging::update_log_level(level);
}
pub mod platform;
pub mod presenter;
pub mod services;
pub mod surface;
