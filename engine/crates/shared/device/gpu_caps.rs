//! Per-session GPU compressed texture format support.
//!
//! `GpuCaps` is created by the host thread and shared with the render
//! thread via `Arc`.  The render thread calls `set()` after GL context
//! init; IO/JS threads read via `snapshot()`.
//!
//! `GpuCapsSnapshot` is a plain `Copy` struct passed to image decode
//! functions so that decode decisions use the caps that were current
//! when the request was dispatched.
//!
//! `wait_ready()` blocks until the render thread has completed GL init
//! and called `set()`.  `Host::new()` calls this to ensure caps are
//! populated before any JS code can issue image loads.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug, Clone)]
pub enum GpuCapsReadyState {
    Ready,
    Failed(String),
    Timeout,
}

/// Per-session GPU compressed texture format support.
///
/// Created once per host session, shared with the render thread.
/// The render thread writes after GL context init (via `set`);
/// IO/JS threads read (via `snapshot`).
#[derive(Debug)]
pub struct GpuCaps {
    etc2: AtomicBool,
    astc: AtomicBool,
    /// Set to `true` after `set()` is called.  `wait_ready()` blocks
    /// until this flag is true, ensuring no early snapshot reads
    /// uninitialized (all-false) caps.
    ready: AtomicBool,
    failed: AtomicBool,
    ready_lock: Mutex<bool>,
    ready_cv: Condvar,
    failure_detail: Mutex<Option<String>>,
}

impl Default for GpuCaps {
    fn default() -> Self {
        Self {
            etc2: AtomicBool::new(false),
            astc: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            ready_lock: Mutex::new(false),
            ready_cv: Condvar::new(),
            failure_detail: Mutex::new(None),
        }
    }
}

impl GpuCaps {
    /// Create a new `Arc<GpuCaps>` with both formats defaulting to `false`.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Called by the render thread after GL context init.
    pub fn set(&self, etc2: bool, astc: bool) {
        self.etc2.store(etc2, Ordering::Release);
        self.astc.store(astc, Ordering::Release);
        self.ready.store(true, Ordering::Release);
        if let Ok(mut ready) = self.ready_lock.lock() {
            *ready = true;
            self.ready_cv.notify_all();
        }
    }

    pub fn set_failed(&self, detail: impl Into<String>) {
        self.failed.store(true, Ordering::Release);
        self.ready.store(true, Ordering::Release);
        if let Ok(mut failure) = self.failure_detail.lock() {
            *failure = Some(detail.into());
        }
        if let Ok(mut ready) = self.ready_lock.lock() {
            *ready = true;
            self.ready_cv.notify_all();
        }
    }

    /// Whether `set()` has been called.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// Block until `set()` has been called by the render thread.
    ///
    pub fn wait_ready(&self, timeout: std::time::Duration) -> GpuCapsReadyState {
        if self.ready.load(Ordering::Acquire) {
            if self.failed.load(Ordering::Acquire) {
                let detail = self
                    .failure_detail
                    .lock()
                    .ok()
                    .and_then(|g| g.clone())
                    .unwrap_or_else(|| "GPU caps initialization failed".to_string());
                return GpuCapsReadyState::Failed(detail);
            }
            return GpuCapsReadyState::Ready;
        }
        let Ok(ready) = self.ready_lock.lock() else {
            return GpuCapsReadyState::Timeout;
        };
        if *ready {
            if self.failed.load(Ordering::Acquire) {
                let detail = self
                    .failure_detail
                    .lock()
                    .ok()
                    .and_then(|g| g.clone())
                    .unwrap_or_else(|| "GPU caps initialization failed".to_string());
                return GpuCapsReadyState::Failed(detail);
            }
            return GpuCapsReadyState::Ready;
        }
        match self.ready_cv.wait_timeout_while(ready, timeout, |r| !*r) {
            Ok((guard, _)) if *guard => {
                if self.failed.load(Ordering::Acquire) {
                    let detail = self
                        .failure_detail
                        .lock()
                        .ok()
                        .and_then(|g| g.clone())
                        .unwrap_or_else(|| "GPU caps initialization failed".to_string());
                    GpuCapsReadyState::Failed(detail)
                } else {
                    GpuCapsReadyState::Ready
                }
            }
            Ok(_) | Err(_) => GpuCapsReadyState::Timeout,
        }
    }

    /// Take an immutable point-in-time snapshot for passing to image decode functions.
    pub fn snapshot(&self) -> GpuCapsSnapshot {
        GpuCapsSnapshot {
            etc2: self.etc2.load(Ordering::Acquire),
            astc: self.astc.load(Ordering::Acquire),
        }
    }
}

/// Immutable point-in-time copy of GPU caps.
///
/// Passed to image decode functions so decode decisions use the caps
/// that were current when the request was dispatched.
#[derive(Debug, Clone, Copy, Default)]
pub struct GpuCapsSnapshot {
    pub etc2: bool,
    pub astc: bool,
}

#[cfg(test)]
mod tests {
    use super::{GpuCaps, GpuCapsReadyState};
    use std::time::Duration;

    #[test]
    fn wait_ready_observes_set() {
        let caps = GpuCaps::new();
        let worker = caps.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            worker.set(true, false);
        });
        assert!(matches!(
            caps.wait_ready(Duration::from_secs(1)),
            GpuCapsReadyState::Ready
        ));
        let snap = caps.snapshot();
        assert!(snap.etc2);
        assert!(!snap.astc);
    }

    #[test]
    fn wait_ready_times_out_when_unset() {
        let caps = GpuCaps::new();
        assert!(matches!(
            caps.wait_ready(Duration::from_millis(1)),
            GpuCapsReadyState::Timeout
        ));
    }

    #[test]
    fn wait_ready_returns_failure() {
        let caps = GpuCaps::new();
        caps.set_failed("boom");
        match caps.wait_ready(Duration::from_millis(1)) {
            GpuCapsReadyState::Failed(msg) => assert!(msg.contains("boom")),
            other => panic!("expected failure, got {other:?}"),
        }
    }
}
