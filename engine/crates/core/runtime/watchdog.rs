//! # ANR Watchdog
//!
//! Monitors the host JS thread for liveness and terminates the V8 isolate
//! if the thread becomes unresponsive (e.g., infinite loop in JS code).
//!
//! ## Design
//!
//! - The host thread writes a monotonic timestamp (millis since watchdog
//!   construction) to a shared `AtomicU64` on every event loop iteration
//!   ("heartbeat").  Using `Instant` instead of `SystemTime` avoids false
//!   positives from NTP corrections or device sleep on ARM/Android.
//! - A lightweight watchdog thread periodically reads this value.  If the gap
//!   exceeds `timeout`, the V8 isolate is terminated via
//!   `IsolateHandle::terminate_execution()`, an ANR is reported, and telemetry
//!   is emitted.
//! - The watchdog thread exits cleanly when the host thread drops the
//!   [`WatchdogHandle`] (the `alive` flag is set to `false`).
//!
//! ## Pause / Resume (False-Positive Control)
//!
//! Some operations legitimately block for extended periods (e.g., synchronous
//! file I/O, module compilation).  Before entering such a section, the host
//! thread should call [`WatchdogState::pause()`] which temporarily suspends
//! timeout checking.  Call [`WatchdogState::resume()`] when done.
//!
//! Alternatively, [`WatchdogState::extend_timeout()`] can temporarily increase
//! the timeout for a specific long-running task without fully disabling the
//! watchdog.
//!
//! ## Telemetry
//!
//! The watchdog provides a pluggable [`TelemetryReporter`] trait.  Callers can
//! supply a custom implementation to report ANR events to their analytics
//! backend.  A no-op default is used when none is configured.
//!
//! ## Feature Gate
//!
//! All code in this module is gated behind `cfg(feature = "v8-limits")`.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use deno_core::v8;
use shared::error::{EngineError, EngineResult, ErrorCode};
use tracing::{info, warn, error};

/// How often the watchdog thread checks the heartbeat.
/// Default: every 2 seconds (configurable via [`WatchdogConfig`]).
const DEFAULT_CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// Default ANR timeout: 10 seconds without a heartbeat.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Termination Reason
// ---------------------------------------------------------------------------

/// Reason why the watchdog terminated the isolate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationReason {
    /// JS execution exceeded the configured timeout without yielding.
    Timeout,
    /// V8 heap limit was exceeded (near-heap-limit callback fired).
    OutOfMemory,
}

// ---------------------------------------------------------------------------
// Telemetry
// ---------------------------------------------------------------------------

/// Trait for reporting ANR / watchdog events to an analytics backend.
///
/// Implement this trait and pass an `Arc<dyn TelemetryReporter>` to
/// [`WatchdogConfig`] to receive ANR reports.
pub trait TelemetryReporter: Send + Sync {
    /// Called when the watchdog detects an ANR.
    ///
    /// # Arguments
    /// * `host_id` - The host ID that triggered the ANR.
    /// * `reason` - Why the isolate was terminated.
    /// * `elapsed_ms` - How many milliseconds elapsed since the last heartbeat.
    /// * `backtrace` - Best-effort backtrace (may be empty on release builds).
    fn report_anr(
        &self,
        host_id: i32,
        reason: TerminationReason,
        elapsed_ms: u64,
        backtrace: &str,
    );
}

/// No-op telemetry reporter (default).
struct NoopReporter;

impl TelemetryReporter for NoopReporter {
    fn report_anr(&self, _host_id: i32, _reason: TerminationReason, _elapsed_ms: u64, _backtrace: &str) {
        // Intentionally empty - logging is handled by the watchdog loop itself.
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the ANR watchdog.
#[derive(Clone)]
pub struct WatchdogConfig {
    /// Maximum time between heartbeats before an ANR is triggered.
    /// Default: 10 seconds.
    pub timeout: Duration,
    /// How often the watchdog thread wakes up to check the heartbeat.
    /// Default: 2 seconds.  Should be < timeout.
    pub check_interval: Duration,
    /// Optional telemetry reporter for ANR events.
    pub telemetry: Arc<dyn TelemetryReporter>,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            check_interval: DEFAULT_CHECK_INTERVAL,
            telemetry: Arc::new(NoopReporter),
        }
    }
}

impl WatchdogConfig {
    /// Create a new config with the given ANR timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the check interval.
    pub fn with_check_interval(mut self, interval: Duration) -> Self {
        self.check_interval = interval;
        self
    }

    /// Set a custom telemetry reporter.
    pub fn with_telemetry(mut self, reporter: Arc<dyn TelemetryReporter>) -> Self {
        self.telemetry = reporter;
        self
    }
}

// ---------------------------------------------------------------------------
// Shared State
// ---------------------------------------------------------------------------

/// Shared state between the host thread and the watchdog thread.
///
/// All timing uses monotonic `Instant`-based offsets (millis since an epoch
/// captured at construction) instead of wall-clock `SystemTime`.  This avoids
/// false positives from NTP corrections or device sleep on Android.
pub struct WatchdogState {
    /// Monotonic epoch captured once at construction.
    /// All `heartbeat` / `extended_deadline` values are ms offsets from this.
    epoch: Instant,
    /// Monotonic millis offset of the last heartbeat from the host thread.
    pub heartbeat: AtomicU64,
    /// Set to `true` when the watchdog terminates the isolate.
    pub terminated: AtomicBool,
    /// The reason for termination, if any.
    /// Encoded as u8: 0 = none, 1 = Timeout, 2 = OOM.
    pub termination_reason: AtomicU64,
    /// When true, the watchdog skips timeout checking.
    /// Used for long-running operations that legitimately block the event loop.
    paused: AtomicBool,
    /// Temporary timeout override (monotonic millis offset when the extended
    /// deadline expires).  0 means no override is active.
    extended_deadline: AtomicU64,
    /// Set to `false` to ask the watchdog thread to exit.
    alive: AtomicBool,
}

impl WatchdogState {
    fn new() -> Self {
        let epoch = Instant::now();
        Self {
            epoch,
            heartbeat: AtomicU64::new(0), // offset 0 = "just created"
            terminated: AtomicBool::new(false),
            termination_reason: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            extended_deadline: AtomicU64::new(0),
            alive: AtomicBool::new(true),
        }
    }

    /// Monotonic millis since the internal epoch.
    #[inline]
    fn mono_millis(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    /// Update the heartbeat timestamp. Must be called from the host thread
    /// on every event loop iteration.
    #[inline]
    pub fn tick(&self) {
        self.heartbeat.store(self.mono_millis(), Ordering::Release);
    }

    /// Record that execution was terminated and the reason.
    pub fn mark_terminated(&self, reason: TerminationReason) {
        let code = match reason {
            TerminationReason::Timeout => 1,
            TerminationReason::OutOfMemory => 2,
        };
        self.termination_reason.store(code, Ordering::SeqCst);
        self.terminated.store(true, Ordering::SeqCst);
    }

    /// Check if the isolate was terminated by the watchdog or OOM callback.
    #[inline]
    pub fn was_terminated(&self) -> bool {
        self.terminated.load(Ordering::SeqCst)
    }

    /// Return the termination reason if any.
    pub fn termination_reason(&self) -> Option<TerminationReason> {
        match self.termination_reason.load(Ordering::SeqCst) {
            1 => Some(TerminationReason::Timeout),
            2 => Some(TerminationReason::OutOfMemory),
            _ => None,
        }
    }

    /// Reset termination state (used after `cancel_terminate_execution` for restart).
    pub fn reset(&self) {
        self.terminated.store(false, Ordering::SeqCst);
        self.termination_reason.store(0, Ordering::SeqCst);
        self.paused.store(false, Ordering::Relaxed);
        self.extended_deadline.store(0, Ordering::Relaxed);
        self.heartbeat.store(self.mono_millis(), Ordering::Release);
    }

    // ---- Pause / Resume (false-positive control) ----

    /// Temporarily pause the watchdog.
    ///
    /// Call this before entering a legitimately long-running operation
    /// (e.g., synchronous module compilation, large file I/O) to prevent
    /// false ANR reports.
    ///
    /// **Important:** Always call [`resume()`] when done.
    #[inline]
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
    }

    /// Resume watchdog monitoring after a pause.
    ///
    /// Also refreshes the heartbeat so the elapsed-time counter resets.
    #[inline]
    pub fn resume(&self) {
        self.heartbeat.store(self.mono_millis(), Ordering::Release);
        self.paused.store(false, Ordering::Release);
    }

    /// Whether the watchdog is currently paused.
    #[inline]
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    // ---- Extended Timeout (long-task whitelist) ----

    /// Temporarily extend the watchdog timeout for a known long-running task.
    ///
    /// The watchdog will not fire until `now + extra` even if the normal
    /// timeout would have expired.  Unlike [`pause()`], the watchdog still
    /// runs; it just uses a later deadline.
    ///
    /// # Arguments
    /// * `extra` - How much additional time to allow beyond the current moment.
    pub fn extend_timeout(&self, extra: Duration) {
        let deadline = self.mono_millis() + extra.as_millis() as u64;
        self.extended_deadline.store(deadline, Ordering::Release);
    }

    /// Clear any temporary timeout extension.
    ///
    /// Also refreshes the heartbeat.
    pub fn clear_timeout_extension(&self) {
        self.extended_deadline.store(0, Ordering::Release);
        self.heartbeat.store(self.mono_millis(), Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// Watchdog Handle
// ---------------------------------------------------------------------------

/// Handle returned by [`spawn_watchdog`]. Dropping it stops the watchdog thread.
pub struct WatchdogHandle {
    pub state: Arc<WatchdogState>,
    _thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for WatchdogHandle {
    fn drop(&mut self) {
        self.state.alive.store(false, Ordering::SeqCst);
        // The watchdog thread will exit on its next check interval.
        // We don't join here to avoid blocking the host thread during shutdown.
    }
}

// ---------------------------------------------------------------------------
// Spawn
// ---------------------------------------------------------------------------

/// Spawn a watchdog thread that monitors the given `v8::IsolateHandle`.
///
/// # Arguments
///
/// * `isolate_handle` - A thread-safe handle to the V8 isolate.
/// * `config` - Watchdog configuration (timeout, check interval, telemetry).
/// * `host_id` - Host ID for logging.
///
/// # Returns
///
/// A [`WatchdogHandle`] whose `.state` must be ticked from the host thread.
pub fn spawn_watchdog(
    isolate_handle: v8::IsolateHandle,
    config: WatchdogConfig,
    host_id: i32,
) -> EngineResult<WatchdogHandle> {
    let state = Arc::new(WatchdogState::new());
    let state_clone = Arc::clone(&state);

    let thread = std::thread::Builder::new()
        .name(format!("Migo-Watchdog-{}", host_id))
        .spawn(move || {
            watchdog_loop(state_clone, isolate_handle, config, host_id);
        })
        .map_err(|e| {
            EngineError::new(ErrorCode::Internal)
                .with_msg("failed to spawn watchdog thread")
                .with_detail(e.to_string())
        })?;

    Ok(WatchdogHandle {
        state,
        _thread: Some(thread),
    })
}

// ---------------------------------------------------------------------------
// Main Loop
// ---------------------------------------------------------------------------

fn watchdog_loop(
    state: Arc<WatchdogState>,
    isolate_handle: v8::IsolateHandle,
    config: WatchdogConfig,
    host_id: i32,
) {
    let timeout_ms = config.timeout.as_millis() as u64;

    info!(
        "[Host {}] Watchdog started: timeout={}s, check_interval={}s",
        host_id,
        config.timeout.as_secs(),
        config.check_interval.as_secs(),
    );

    while state.alive.load(Ordering::SeqCst) {
        std::thread::sleep(config.check_interval);

        if !state.alive.load(Ordering::SeqCst) {
            break;
        }

        // Already terminated, no need to check again
        if state.terminated.load(Ordering::SeqCst) {
            continue;
        }

        // Paused: skip this check cycle
        if state.paused.load(Ordering::Acquire) {
            continue;
        }

        let now = state.mono_millis();

        // Check for temporary timeout extension
        let extended_deadline = state.extended_deadline.load(Ordering::Acquire);
        if extended_deadline > 0 && now < extended_deadline {
            // Still within the extended deadline window, skip this check.
            continue;
        }

        let last = state.heartbeat.load(Ordering::Acquire);
        let elapsed = now.saturating_sub(last);

        if elapsed > timeout_ms {
            warn!(
                "[Host {}] Watchdog: ANR detected! JS execution unresponsive for {}ms (timeout={}ms), terminating isolate",
                host_id, elapsed, timeout_ms
            );

            // Best-effort backtrace capture
            let backtrace = capture_backtrace();

            state.mark_terminated(TerminationReason::Timeout);
            isolate_handle.terminate_execution();

            // NOTE: We intentionally do NOT call `platform.notify_error()` here.
            // The host thread in thread.rs detects the termination via
            // `classify_termination_error()` and issues a single notification
            // to Java, avoiding duplicate callbacks.  Telemetry is fired here
            // for analytics only (no user-facing side-effects).
            config.telemetry.report_anr(
                host_id,
                TerminationReason::Timeout,
                elapsed,
                &backtrace,
            );

            error!(
                "[Host {}] Watchdog: ANR reported. elapsed={}ms, backtrace:\n{}",
                host_id, elapsed, if backtrace.is_empty() { "(not available)" } else { &backtrace }
            );

            break; // Exit after termination
        }
    }

    info!("[Host {}] Watchdog thread exiting", host_id);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Best-effort backtrace capture.
///
/// On debug builds this returns a formatted backtrace.
/// On release builds (with `strip = "symbols"`) it will be mostly empty.
fn capture_backtrace() -> String {
    // std::backtrace is stabilized since Rust 1.65.
    // On release builds with stripped symbols, this will have limited info
    // but it's still useful on debug/non-stripped builds.
    let bt = std::backtrace::Backtrace::force_capture();
    format!("{}", bt)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watchdog_state_initial() {
        let state = WatchdogState::new();
        assert!(!state.was_terminated());
        assert!(state.termination_reason().is_none());
        assert!(!state.is_paused());
    }

    #[test]
    fn test_watchdog_state_mark_terminated_timeout() {
        let state = WatchdogState::new();
        state.mark_terminated(TerminationReason::Timeout);
        assert!(state.was_terminated());
        assert_eq!(state.termination_reason(), Some(TerminationReason::Timeout));
    }

    #[test]
    fn test_watchdog_state_mark_terminated_oom() {
        let state = WatchdogState::new();
        state.mark_terminated(TerminationReason::OutOfMemory);
        assert!(state.was_terminated());
        assert_eq!(state.termination_reason(), Some(TerminationReason::OutOfMemory));
    }

    #[test]
    fn test_watchdog_state_reset() {
        let state = WatchdogState::new();
        state.mark_terminated(TerminationReason::Timeout);
        state.pause();
        state.extend_timeout(Duration::from_secs(60));
        assert!(state.was_terminated());
        assert!(state.is_paused());

        state.reset();
        assert!(!state.was_terminated());
        assert!(state.termination_reason().is_none());
        assert!(!state.is_paused());
        assert_eq!(state.extended_deadline.load(Ordering::Acquire), 0);
    }

    #[test]
    fn test_watchdog_state_tick_updates_heartbeat() {
        let state = WatchdogState::new();
        let before = state.heartbeat.load(std::sync::atomic::Ordering::Acquire);
        std::thread::sleep(std::time::Duration::from_millis(10));
        state.tick();
        let after = state.heartbeat.load(std::sync::atomic::Ordering::Acquire);
        assert!(after > before, "Heartbeat should increase after tick");
    }

    #[test]
    fn test_watchdog_pause_resume() {
        let state = WatchdogState::new();
        assert!(!state.is_paused());

        state.pause();
        assert!(state.is_paused());

        state.resume();
        assert!(!state.is_paused());
    }

    #[test]
    fn test_watchdog_extend_timeout() {
        let state = WatchdogState::new();
        assert_eq!(state.extended_deadline.load(Ordering::Acquire), 0);

        state.extend_timeout(Duration::from_secs(30));
        let deadline = state.extended_deadline.load(Ordering::Acquire);
        assert!(deadline > 0);

        state.clear_timeout_extension();
        assert_eq!(state.extended_deadline.load(Ordering::Acquire), 0);
    }

    #[test]
    fn test_mono_millis_is_monotonic() {
        let state = WatchdogState::new();
        let a = state.mono_millis();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let b = state.mono_millis();
        assert!(b > a, "mono_millis should be monotonically increasing");
    }

    #[test]
    fn test_watchdog_config_default() {
        let config = WatchdogConfig::default();
        assert_eq!(config.timeout, Duration::from_secs(10));
        assert_eq!(config.check_interval, Duration::from_secs(2));
    }

    #[test]
    fn test_watchdog_config_builder() {
        let config = WatchdogConfig::default()
            .with_timeout(Duration::from_secs(30))
            .with_check_interval(Duration::from_secs(5));
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.check_interval, Duration::from_secs(5));
    }

    #[test]
    fn test_telemetry_reporter_called() {
        use std::sync::atomic::AtomicU32;

        struct CountingReporter {
            count: AtomicU32,
        }
        impl TelemetryReporter for CountingReporter {
            fn report_anr(&self, _host_id: i32, _reason: TerminationReason, _elapsed_ms: u64, _backtrace: &str) {
                self.count.fetch_add(1, Ordering::Relaxed);
            }
        }

        let reporter = Arc::new(CountingReporter { count: AtomicU32::new(0) });
        reporter.report_anr(1, TerminationReason::Timeout, 12000, "");
        assert_eq!(reporter.count.load(Ordering::Relaxed), 1);
    }
}
