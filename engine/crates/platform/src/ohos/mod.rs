//! OpenHarmony platform support.
//!
//! Scoped to what a native host needs to hand the engine a drawable surface:
//! the `OHNativeWindow` wrapper and the EGL presenter built on it. Device
//! services, the frame clock and host notifications are not here — an
//! OpenHarmony host reaches those through the C ABI's host-callback channel,
//! the same way the pure-native Android host does, rather than through a
//! platform-owned services bundle.

pub mod presenter;
pub mod surface;
