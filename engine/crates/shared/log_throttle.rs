//! Rate-limited / once-only wrappers around `tracing` macros.
//!
//! The engine intentionally emits `tracing::warn!` on a few hot
//! error paths (AHB fallback, unresolved font family, partial-update
//! probe failures, …) because silently swallowing the condition
//! hides real regressions.  Under sustained failure (a driver that
//! permanently rejects AHB import, a game that registers 30+ fonts
//! at startup, or a RAF-saturated event loop), the naive approach
//! of one `warn!` per occurrence produces a log-storm at hundreds
//! of events per second — the `tracing` formatter allocates ~300
//! bytes per event, so the hot path pays a measurable CPU + GC
//! tax on Android where `tracing-android` round-trips through
//! `__android_log_print`.
//!
//! This module provides two small helpers:
//!
//! 1. [`warn_once!`] — emits the first call through and drops every
//!    subsequent call at the same source location.  Zero-allocation
//!    steady state (a single `AtomicBool` check).
//! 2. [`warn_rate_limited!`] — emits at most once per specified
//!    interval (default 1 second) at the same source location.
//!    Useful when the operator still benefits from periodic hints
//!    that the condition persists, just not once per frame.
//!
//! Both macros are **per call site** — the identity is the file /
//! line / column of the invocation, so two separate `warn_once!`
//! lines emit independently.  They behave exactly like a plain
//! `tracing::warn!` on first emit, including the target / fields
//! / format-string semantics of the underlying macro.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Primitive backing [`warn_once!`].  Exposed so the macro can
/// name the storage type without requiring users of the macro to
/// import this module.
#[doc(hidden)]
pub struct OnceFlag {
    fired: AtomicBool,
}

impl OnceFlag {
    /// `const`-construct so the macro can place one of these in a
    /// `static`.
    pub const fn new() -> Self {
        Self {
            fired: AtomicBool::new(false),
        }
    }

    /// Returns `true` exactly once per instance, ignoring any
    /// subsequent calls.
    pub fn should_emit(&self) -> bool {
        // `Relaxed` is sufficient: we don't care which thread wins
        // the race, only that subsequent calls observe `true`.  The
        // observable effect is a log line, so a small duplicate
        // window under contention is harmless.
        !self.fired.swap(true, Ordering::Relaxed)
    }
}

/// Primitive backing [`warn_rate_limited!`].  Stores the last emit
/// time as nanoseconds since a process-wide monotonic epoch.
#[doc(hidden)]
pub struct RateGate {
    last_emit_ns: AtomicU64,
    min_interval_nanos: u64,
}

impl RateGate {
    pub const fn new(min_interval: Duration) -> Self {
        Self {
            last_emit_ns: AtomicU64::new(0),
            // `Duration::as_nanos` isn't `const` on stable for
            // some toolchains, but `as_secs * 1e9 + subsec_nanos`
            // is; we accept a small truncation for durations >
            // 292 years (`u64::MAX` nanoseconds).
            min_interval_nanos: min_interval.as_secs() * 1_000_000_000
                + min_interval.subsec_nanos() as u64,
        }
    }

    pub fn should_emit(&self) -> bool {
        use std::sync::OnceLock;
        static EPOCH: OnceLock<Instant> = OnceLock::new();
        let epoch = *EPOCH.get_or_init(Instant::now);
        let now_ns = epoch.elapsed().as_nanos() as u64;
        let last = self.last_emit_ns.load(Ordering::Relaxed);
        if now_ns.saturating_sub(last) < self.min_interval_nanos {
            return false;
        }
        // Compare-and-swap so only one thread wins the race on
        // transitions; the rest suppress.  If CAS fails the race,
        // suppress too (someone else just emitted).
        self.last_emit_ns
            .compare_exchange(last, now_ns, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    }
}

/// Emit a `tracing::warn!` only the first time the macro is
/// reached at this source location.  Identical syntax to
/// `tracing::warn!`; the first call dispatches to it, the rest
/// are ~1 ns no-ops (one `AtomicBool::swap`).
///
/// ```ignore
/// warn_once!("AHB EGLImage import failed; falling back to RGBA+PBO");
/// ```
#[macro_export]
macro_rules! warn_once {
    ($($arg:tt)+) => {{
        static __MIGO_WARN_ONCE: $crate::log_throttle::OnceFlag = $crate::log_throttle::OnceFlag::new();
        if __MIGO_WARN_ONCE.should_emit() {
            ::tracing::warn!($($arg)+);
        }
    }};
}

/// Emit a `tracing::warn!` at most once per configured interval
/// at this source location.  First argument is the interval (as
/// a `std::time::Duration`), the rest are forwarded to
/// `tracing::warn!` verbatim.
///
/// ```ignore
/// warn_rate_limited!(
///     Duration::from_secs(1),
///     "RAF drop streak = {streak}"
/// );
/// ```
#[macro_export]
macro_rules! warn_rate_limited {
    ($interval:expr, $($arg:tt)+) => {{
        static __MIGO_WARN_GATE: $crate::log_throttle::RateGate =
            $crate::log_throttle::RateGate::new($interval);
        if __MIGO_WARN_GATE.should_emit() {
            ::tracing::warn!($($arg)+);
        }
    }};
}

/// Emit a `tracing::info!` only the first time the macro is
/// reached at this source location.  Useful for "first time we
/// saw X" diagnostics — e.g. first typeface-resolution for a
/// family, first frame presented after surface recreate — where
/// every subsequent occurrence adds no new information.
#[macro_export]
macro_rules! info_once {
    ($($arg:tt)+) => {{
        static __MIGO_INFO_ONCE: $crate::log_throttle::OnceFlag = $crate::log_throttle::OnceFlag::new();
        if __MIGO_INFO_ONCE.should_emit() {
            ::tracing::info!($($arg)+);
        }
    }};
}

/// Emit a `tracing::info!` at most once per configured interval
/// at this source location.  Paired with [`warn_rate_limited!`]
/// for the less-alarming side of the spectrum — e.g. "LRU is
/// over byte budget because of pins" is an operating-as-intended
/// observation that still benefits from periodic sampling.
#[macro_export]
macro_rules! info_rate_limited {
    ($interval:expr, $($arg:tt)+) => {{
        static __MIGO_INFO_GATE: $crate::log_throttle::RateGate =
            $crate::log_throttle::RateGate::new($interval);
        if __MIGO_INFO_GATE.should_emit() {
            ::tracing::info!($($arg)+);
        }
    }};
}

/// Emit a `tracing::trace!` at most once per configured
/// interval.  Trace is opt-in via `RUST_LOG=…=trace` so runtime
/// cost is zero by default; the rate gate is here so turning it
/// on in production doesn't flood logcat.
#[macro_export]
macro_rules! trace_rate_limited {
    ($interval:expr, $($arg:tt)+) => {{
        static __MIGO_TRACE_GATE: $crate::log_throttle::RateGate =
            $crate::log_throttle::RateGate::new($interval);
        if __MIGO_TRACE_GATE.should_emit() {
            ::tracing::trace!($($arg)+);
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn once_flag_only_fires_once() {
        let f = OnceFlag::new();
        assert!(f.should_emit());
        assert!(!f.should_emit());
        assert!(!f.should_emit());
    }

    #[test]
    fn rate_gate_honours_interval() {
        let g = RateGate::new(Duration::from_millis(30));
        assert!(g.should_emit());
        assert!(
            !g.should_emit(),
            "second call within the gate must be suppressed"
        );
        sleep(Duration::from_millis(50));
        assert!(g.should_emit(), "after the gate elapses emissions resume");
    }
}
