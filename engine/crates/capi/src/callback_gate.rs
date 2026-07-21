//! Linearization gate between host callbacks and Session destruction.
//!
//! Callback entry is a hot path: while the Session is open it performs one
//! atomic compare/exchange and no locking or allocation. Destruction is cold;
//! it closes admission, then waits for callbacks already executing on other
//! threads. Callback frames on the destroying thread are exempt so a callback
//! may destroy its own Session without deadlocking.

use std::{
    cell::Cell,
    ptr,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

const CLOSED_BIT: usize = 1usize << (usize::BITS - 1);
const ACTIVE_MASK: usize = CLOSED_BIT - 1;

struct CallbackFrame {
    gate: *const CallbackGate,
    previous: *const CallbackFrame,
}

thread_local! {
    static CALLBACK_STACK: Cell<*const CallbackFrame> = const { Cell::new(ptr::null()) };
}

/// Restores the thread-local stack even if Rust test code unwinds.
struct StackRestore<'a> {
    stack: &'a Cell<*const CallbackFrame>,
    previous: *const CallbackFrame,
}

impl Drop for StackRestore<'_> {
    fn drop(&mut self) {
        self.stack.set(self.previous);
    }
}

/// Open/closed state and the number of callbacks that won entry before close.
pub(crate) struct CallbackGate {
    state: AtomicUsize,
    drain_lock: Mutex<()>,
    drained: Condvar,
}

impl CallbackGate {
    pub(crate) const fn new() -> Self {
        Self {
            state: AtomicUsize::new(0),
            drain_lock: Mutex::new(()),
            drained: Condvar::new(),
        }
    }

    #[inline]
    pub(crate) fn is_open(&self) -> bool {
        self.state.load(Ordering::Acquire) & CLOSED_BIT == 0
    }

    /// Acquire permission to enter host code if destruction has not closed the
    /// gate. The successful CAS is the callback's linearization point.
    #[inline]
    pub(crate) fn try_enter(&self) -> Option<CallbackPermit<'_>> {
        let mut state = self.state.load(Ordering::Acquire);
        loop {
            if state & CLOSED_BIT != 0 || state & ACTIVE_MASK == ACTIVE_MASK {
                return None;
            }
            match self.state.compare_exchange_weak(
                state,
                state + 1,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(CallbackPermit { gate: self }),
                Err(observed) => state = observed,
            }
        }
    }

    /// Permanently close callback admission.
    ///
    /// The returned drain is deliberately separate: Session destruction calls
    /// `close` while its state locks still establish the logical boundary, then
    /// drops those locks before waiting for callbacks that may re-enter Migo.
    pub(crate) fn close(&self) -> Result<CallbackDrain<'_>, AlreadyClosed> {
        let previous = self.state.fetch_or(CLOSED_BIT, Ordering::AcqRel);
        if previous & CLOSED_BIT != 0 {
            return Err(AlreadyClosed);
        }
        Ok(CallbackDrain {
            gate: self,
            local_depth: self.local_depth(),
        })
    }

    fn local_depth(&self) -> usize {
        CALLBACK_STACK.with(|stack| {
            let mut depth = 0;
            let mut frame = stack.get();
            while !frame.is_null() {
                // SAFETY: every frame is a stack local whose registration is
                // scoped by `CallbackPermit::run`; this traversal is on that
                // same thread and cannot outlive the registered call.
                let current = unsafe { &*frame };
                if ptr::eq(current.gate, self) {
                    depth += 1;
                }
                frame = current.previous;
            }
            depth
        })
    }

    fn active_count(&self) -> usize {
        self.state.load(Ordering::Acquire) & ACTIVE_MASK
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AlreadyClosed;

/// One callback that linearized before Session close.
pub(crate) struct CallbackPermit<'a> {
    gate: &'a CallbackGate,
}

impl CallbackPermit<'_> {
    /// Execute the actual host callback with a zero-allocation TLS frame so a
    /// reentrant Session destroy can identify its own callback stack.
    pub(crate) fn run<R>(self, callback: impl FnOnce() -> R) -> R {
        CALLBACK_STACK.with(|stack| {
            let frame = CallbackFrame {
                gate: self.gate,
                previous: stack.get(),
            };
            stack.set(&frame);
            let _restore = StackRestore {
                stack,
                previous: frame.previous,
            };
            callback()
        })
    }
}

impl Drop for CallbackPermit<'_> {
    fn drop(&mut self) {
        let previous = self.gate.state.fetch_sub(1, Ordering::Release);
        debug_assert_ne!(previous & ACTIVE_MASK, 0, "callback count underflow");
        if previous & CLOSED_BIT != 0 {
            // The mutex is needed only after close. Taking it around notify
            // closes the classic predicate-check/condvar-sleep lost-wakeup
            // window without penalizing normal callback delivery.
            let _drain_guard = self
                .gate
                .drain_lock
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            self.gate.drained.notify_all();
        }
    }
}

/// Proof that callback admission is closed, carrying the number of callback
/// frames that must remain live until a reentrant destroy returns.
pub(crate) struct CallbackDrain<'a> {
    gate: &'a CallbackGate,
    local_depth: usize,
}

impl CallbackDrain<'_> {
    pub(crate) fn wait(self) {
        if self.gate.active_count() <= self.local_depth {
            return;
        }

        let mut guard = self
            .gate
            .drain_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        while self.gate.active_count() > self.local_depth {
            guard = self
                .gate
                .drained
                .wait(guard)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, mpsc},
        time::Duration,
    };

    use super::CallbackGate;

    #[test]
    fn close_rejects_new_callbacks_and_waits_for_another_thread() {
        let gate = Arc::new(CallbackGate::new());
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (drained_tx, drained_rx) = mpsc::channel();

        std::thread::scope(|scope| {
            let callback_gate = Arc::clone(&gate);
            scope.spawn(move || {
                callback_gate.try_enter().expect("open gate").run(|| {
                    entered_tx.send(()).expect("signal entry");
                    release_rx.recv().expect("release callback");
                });
            });

            entered_rx.recv().expect("callback entered");
            let drain = gate.close().expect("first close wins");
            assert!(gate.try_enter().is_none(), "close rejects later entry");

            scope.spawn(move || {
                drain.wait();
                drained_tx.send(()).expect("signal drain");
            });
            assert!(
                drained_rx.recv_timeout(Duration::from_millis(25)).is_err(),
                "destroy must not pass a callback running on another thread",
            );

            release_tx.send(()).expect("release callback");
            drained_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("drain completes after callback return");
        });
    }

    #[test]
    fn reentrant_close_exempts_the_complete_current_callback_stack() {
        let gate = CallbackGate::new();
        gate.try_enter().expect("outer callback").run(|| {
            gate.try_enter().expect("nested callback").run(|| {
                let drain = gate.close().expect("first close wins");
                drain.wait();
                assert!(!gate.is_open());
            });
        });
    }

    #[test]
    fn close_is_single_winner() {
        let gate = CallbackGate::new();
        gate.close().expect("first close").wait();
        assert!(gate.close().is_err());
    }
}
