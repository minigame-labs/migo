//! Fixed-capacity ownership pool for message payloads.
//!
//! The slots are heap-allocated once on the cold construction path. Acquiring
//! and returning a slot is a bounded, non-blocking channel operation; there is
//! deliberately no allocation fallback when every slot is in flight.

use std::{fmt, mem::MaybeUninit, ops::Deref, sync::Arc};

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError};

struct Slot<T> {
    value: MaybeUninit<T>,
}

struct PoolInner<T> {
    free_tx: Sender<Box<Slot<T>>>,
    free_rx: Receiver<Box<Slot<T>>>,
}

/// Cloneable handle to a fixed number of reusable payload allocations.
pub struct PayloadPool<T> {
    inner: Arc<PoolInner<T>>,
}

impl<T> Clone for PayloadPool<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> PayloadPool<T> {
    /// Allocate every slot up front.
    ///
    /// # Panics
    /// Panics when `capacity` is zero; a zero-capacity ownership pool cannot
    /// ever make progress and is therefore a construction error.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "payload pool capacity must be non-zero");
        let (free_tx, free_rx) = crossbeam_channel::bounded(capacity);
        for _ in 0..capacity {
            free_tx
                .try_send(Box::new(Slot {
                    value: MaybeUninit::uninit(),
                }))
                .expect("a freshly created pool has exactly capacity slots");
        }
        Self {
            inner: Arc::new(PoolInner { free_tx, free_rx }),
        }
    }

    /// Move `value` into a free slot without blocking.
    ///
    /// Exhaustion returns the unchanged value to the caller. It never grows
    /// the pool and never allocates a replacement slot.
    #[inline]
    pub fn try_insert(&self, value: T) -> Result<Pooled<T>, T> {
        let mut slot = match self.inner.free_rx.try_recv() {
            Ok(slot) => slot,
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Err(value),
        };
        slot.value.write(value);
        Ok(Pooled {
            slot: Some(slot),
            pool: Arc::clone(&self.inner),
        })
    }
}

/// One initialized payload slot.
///
/// Dropping it destroys the current value and returns the allocation to its
/// originating pool, including when a containing channel command is rejected
/// or discarded during shutdown.
pub struct Pooled<T> {
    slot: Option<Box<Slot<T>>>,
    pool: Arc<PoolInner<T>>,
}

impl<T> Deref for Pooled<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        // SAFETY: `try_insert` initializes the slot before constructing this
        // value, and only Drop takes it back out.
        unsafe {
            self.slot
                .as_ref()
                .expect("a live pooled payload owns its slot")
                .value
                .assume_init_ref()
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for Pooled<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.deref().fmt(formatter)
    }
}

impl<T> Drop for Pooled<T> {
    fn drop(&mut self) {
        let mut slot = self
            .slot
            .take()
            .expect("a pooled payload returns its slot exactly once");
        // SAFETY: the slot is initialized for the entire Pooled lifetime and
        // becomes uninitialized again before it is returned to the free list.
        unsafe { slot.value.assume_init_drop() };
        match self.pool.free_tx.try_send(slot) {
            Ok(()) => {}
            // Both cases indicate an invariant failure or final teardown. The
            // value has already been dropped, so releasing the empty allocation
            // is safe and preferable to panicking in Drop.
            Err(TrySendError::Full(slot) | TrySendError::Disconnected(slot)) => drop(slot),
        }
    }
}
