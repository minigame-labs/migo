//! Thread-safe wakeup primitives for cross-thread signaling.
//!
//! Provides [`ThreadWakeup`], a condvar-based wakeup handle that allows
//! one thread to wake another from a timed sleep. Used by the audio thread's
//! power management to enable instant wakeup from deep-sleep states when
//! a new command arrives.

use std::fmt;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// A condvar-based thread wakeup handle.
///
/// When a thread is sleeping via [`wait_timeout`](ThreadWakeup::wait_timeout),
/// another thread can call [`notify`](ThreadWakeup::notify) to wake it
/// immediately, regardless of the remaining timeout.
///
/// This is `Clone` and `Send + Sync` — multiple producers can hold clones
/// and wake the single consumer.
#[derive(Clone)]
pub struct ThreadWakeup {
    inner: Arc<(Mutex<bool>, Condvar)>,
}

impl ThreadWakeup {
    /// Create a new wakeup handle (initially not signaled).
    pub fn new() -> Self {
        Self {
            inner: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    /// Signal the sleeping thread to wake up.
    ///
    /// If the thread is currently in [`wait_timeout`](ThreadWakeup::wait_timeout),
    /// it will return immediately. If it is not sleeping, the signal is
    /// latched and the next `wait_timeout` call will return instantly.
    #[inline]
    pub fn notify(&self) {
        let (lock, cvar) = &*self.inner;
        // Recover from poisoned mutex — if the peer thread panicked while
        // holding this lock we still want to deliver the wakeup signal
        // rather than cascading the panic to this thread.
        let mut signaled = lock.lock().unwrap_or_else(|e| e.into_inner());
        *signaled = true;
        cvar.notify_one();
    }

    /// Wait for a signal or timeout.
    ///
    /// Returns `true` in all cases (signaled or timed out) — the caller
    /// should always proceed to check its work queue after waking.
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        let (lock, cvar) = &*self.inner;
        // Recover from poisoned mutex rather than panicking — this prevents
        // a panic in the audio thread from cascading to the host thread.
        let mut signaled = lock.lock().unwrap_or_else(|e| e.into_inner());
        if *signaled {
            *signaled = false;
            return true;
        }
        let result = cvar
            .wait_timeout(signaled, timeout)
            .unwrap_or_else(|e| e.into_inner());
        signaled = result.0;
        let was_signaled = *signaled;
        *signaled = false;
        was_signaled || result.1.timed_out()
    }
}

impl Default for ThreadWakeup {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ThreadWakeup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ThreadWakeup").finish()
    }
}
