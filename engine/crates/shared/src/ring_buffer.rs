//! Single-producer single-consumer (SPSC) lock-free ring buffer.
//!
//! Designed for the JS→Render command hot path where allocation-free,
//! cache-friendly transport matters.
//!
//! - Fixed capacity (power of 2) for bitwise modulo.
//! - Producer blocks on Condvar when full (not busy-yield).
//! - Consumer drains in batch, notifies producer after drain.
//! - Metrics for debug overlay / observability.

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// Metrics exposed for debug overlay.
pub struct RingMetrics {
    /// Number of times the producer had to block (ring full).
    pub flush_count: AtomicU64,
    /// High-water mark of pending items.
    pub max_pending: AtomicU64,
    /// Total items pushed.
    pub total_pushed: AtomicU64,
}

impl RingMetrics {
    fn new() -> Self {
        Self {
            flush_count: AtomicU64::new(0),
            max_pending: AtomicU64::new(0),
            total_pushed: AtomicU64::new(0),
        }
    }
}

/// Cache-line padded atomic to avoid false sharing between producer and consumer.
#[repr(align(64))]
struct Padded<T>(T);

/// Fixed-size SPSC ring buffer.
///
/// `CAPACITY` must be a power of 2.
pub struct SpscRing<T, const CAPACITY: usize> {
    slots: Box<[UnsafeCell<MaybeUninit<T>>; CAPACITY]>,
    write_pos: Padded<AtomicU64>,
    read_pos: Padded<AtomicU64>,
    /// Condvar for producer blocking when ring is full.
    not_full: Condvar,
    not_full_mutex: Mutex<()>,
    pub metrics: RingMetrics,
}

// Safety: only one producer and one consumer thread access the ring.
// The producer writes to slots[write_pos] and the consumer reads from
// slots[read_pos].  Atomic ordering on the positions ensures visibility.
unsafe impl<T: Send, const CAPACITY: usize> Send for SpscRing<T, CAPACITY> {}
unsafe impl<T: Send, const CAPACITY: usize> Sync for SpscRing<T, CAPACITY> {}

impl<T, const CAPACITY: usize> SpscRing<T, CAPACITY> {
    /// Create a new ring buffer.  `CAPACITY` must be a power of 2.
    pub fn new() -> Self {
        assert!(CAPACITY.is_power_of_two(), "CAPACITY must be a power of 2");
        assert!(CAPACITY > 0);

        // Box::new([UnsafeCell::new(MaybeUninit::uninit()); CAPACITY]) doesn't work
        // for non-Copy types, so we use a Vec→Box conversion.
        let mut slots = Vec::with_capacity(CAPACITY);
        for _ in 0..CAPACITY {
            slots.push(UnsafeCell::new(MaybeUninit::uninit()));
        }
        let slots: Box<[UnsafeCell<MaybeUninit<T>>; CAPACITY]> = slots
            .into_boxed_slice()
            .try_into()
            .ok()
            .expect("Vec length matches CAPACITY");

        Self {
            slots,
            write_pos: Padded(AtomicU64::new(0)),
            read_pos: Padded(AtomicU64::new(0)),
            not_full: Condvar::new(),
            not_full_mutex: Mutex::new(()),
            metrics: RingMetrics::new(),
        }
    }

    /// Push an item.  If the ring is full, blocks until space is available
    /// (Condvar wait, 2ms timeout fallback to prevent deadlock).
    ///
    /// Called from the **producer** thread only.
    pub fn push(&self, item: T) {
        loop {
            let w = self.write_pos.0.load(Ordering::Relaxed);
            let r = self.read_pos.0.load(Ordering::Acquire);
            if (w - r) < CAPACITY as u64 {
                // Space available — write the slot.
                let idx = (w as usize) & (CAPACITY - 1);
                unsafe {
                    (*self.slots[idx].get()).write(item);
                }
                self.write_pos.0.store(w + 1, Ordering::Release);

                // Update metrics.
                self.metrics.total_pushed.fetch_add(1, Ordering::Relaxed);
                let pending = w + 1 - r;
                let _ = self
                    .metrics
                    .max_pending
                    .fetch_max(pending, Ordering::Relaxed);
                return;
            }

            // Ring full — block on Condvar (not busy-yield).
            self.metrics.flush_count.fetch_add(1, Ordering::Relaxed);
            let guard = self.not_full_mutex.lock().unwrap();
            // Re-check after acquiring lock to handle spurious wakeups.
            let r2 = self.read_pos.0.load(Ordering::Acquire);
            if (w - r2) < CAPACITY as u64 {
                drop(guard);
                continue; // Space freed while we waited for the lock.
            }
            // Wait with 2ms timeout — prevents deadlock if consumer died.
            let _ = self.not_full.wait_timeout(guard, Duration::from_millis(2));
        }
    }

    /// Try to push without blocking.  Returns `Err(item)` if full.
    pub fn try_push(&self, item: T) -> Result<(), T> {
        let w = self.write_pos.0.load(Ordering::Relaxed);
        let r = self.read_pos.0.load(Ordering::Acquire);
        if (w - r) >= CAPACITY as u64 {
            return Err(item);
        }
        let idx = (w as usize) & (CAPACITY - 1);
        unsafe {
            (*self.slots[idx].get()).write(item);
        }
        self.write_pos.0.store(w + 1, Ordering::Release);
        self.metrics.total_pushed.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Drain up to `max` items into `out`.  Returns the number drained.
    ///
    /// Called from the **consumer** thread only.
    pub fn drain(&self, out: &mut Vec<T>, max: usize) -> usize {
        let r = self.read_pos.0.load(Ordering::Relaxed);
        let w = self.write_pos.0.load(Ordering::Acquire);
        let available = (w - r) as usize;
        let count = available.min(max);

        out.reserve(count);
        for i in 0..count {
            let idx = ((r + i as u64) as usize) & (CAPACITY - 1);
            unsafe {
                let item = (*self.slots[idx].get()).assume_init_read();
                out.push(item);
            }
        }
        self.read_pos.0.store(r + count as u64, Ordering::Release);

        if count > 0 {
            self.not_full.notify_one();
        }
        count
    }

    /// Number of items currently in the ring.
    pub fn len(&self) -> usize {
        let w = self.write_pos.0.load(Ordering::Acquire);
        let r = self.read_pos.0.load(Ordering::Acquire);
        (w - r) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T, const CAPACITY: usize> Drop for SpscRing<T, CAPACITY> {
    fn drop(&mut self) {
        // Drop any remaining items.
        let r = *self.read_pos.0.get_mut();
        let w = *self.write_pos.0.get_mut();
        for i in r..w {
            let idx = (i as usize) & (CAPACITY - 1);
            unsafe {
                self.slots[idx].get_mut().assume_init_drop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_drain_basic() {
        let ring = SpscRing::<u32, 8>::new();
        ring.push(1);
        ring.push(2);
        ring.push(3);
        assert_eq!(ring.len(), 3);

        let mut out = Vec::new();
        let n = ring.drain(&mut out, 10);
        assert_eq!(n, 3);
        assert_eq!(out, vec![1, 2, 3]);
        assert!(ring.is_empty());
    }

    #[test]
    fn try_push_full() {
        let ring = SpscRing::<u32, 4>::new();
        assert!(ring.try_push(1).is_ok());
        assert!(ring.try_push(2).is_ok());
        assert!(ring.try_push(3).is_ok());
        assert!(ring.try_push(4).is_ok());
        assert!(ring.try_push(5).is_err()); // full

        let mut out = Vec::new();
        ring.drain(&mut out, 2);
        assert!(ring.try_push(5).is_ok()); // space freed
    }

    #[test]
    fn wraparound() {
        let ring = SpscRing::<u32, 4>::new();
        for round in 0..3 {
            for i in 0..4 {
                ring.push(round * 4 + i);
            }
            let mut out = Vec::new();
            ring.drain(&mut out, 4);
            assert_eq!(out.len(), 4);
        }
    }

    #[test]
    fn drop_remaining() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicUsize;

        let drop_count = Arc::new(AtomicUsize::new(0));

        struct Droppable(Arc<AtomicUsize>);
        impl Drop for Droppable {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        {
            let ring = SpscRing::<Droppable, 8>::new();
            ring.push(Droppable(drop_count.clone()));
            ring.push(Droppable(drop_count.clone()));
            ring.push(Droppable(drop_count.clone()));
            // Ring dropped with 3 items still inside.
        }
        assert_eq!(drop_count.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn concurrent_push_drain() {
        use std::sync::Arc;

        let ring = Arc::new(SpscRing::<u64, 256>::new());
        let ring2 = ring.clone();

        let producer = std::thread::spawn(move || {
            for i in 0..10_000u64 {
                ring2.push(i);
            }
        });

        let mut received = Vec::new();
        let mut buf = Vec::new();
        while received.len() < 10_000 {
            buf.clear();
            ring.drain(&mut buf, 512);
            received.extend_from_slice(&buf);
            if buf.is_empty() {
                std::thread::yield_now();
            }
        }
        producer.join().unwrap();

        // Verify ordering.
        for (i, &val) in received.iter().enumerate() {
            assert_eq!(val, i as u64, "out of order at index {i}");
        }
    }
}
