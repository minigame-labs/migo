use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use shared::payload_pool::PayloadPool;

#[derive(Debug)]
struct DropValue {
    id: usize,
    drops: Arc<AtomicUsize>,
}

impl Drop for DropValue {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn exhaustion_returns_the_original_value_without_a_heap_fallback() {
    let drops = Arc::new(AtomicUsize::new(0));
    let pool = PayloadPool::new(1);
    let first = pool
        .try_insert(DropValue {
            id: 1,
            drops: Arc::clone(&drops),
        })
        .unwrap_or_else(|_| panic!("the preallocated slot must be available"));

    let rejected = pool
        .try_insert(DropValue {
            id: 2,
            drops: Arc::clone(&drops),
        })
        .expect_err("an exhausted pool must reject rather than allocate");

    assert_eq!(first.id, 1);
    assert_eq!(rejected.id, 2);
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    drop(rejected);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}

#[test]
fn dropping_a_payload_drops_its_value_and_returns_the_same_bounded_slot() {
    let drops = Arc::new(AtomicUsize::new(0));
    let pool = PayloadPool::new(1);

    let first = pool
        .try_insert(DropValue {
            id: 1,
            drops: Arc::clone(&drops),
        })
        .unwrap_or_else(|_| panic!("slot available"));
    drop(first);
    assert_eq!(drops.load(Ordering::Relaxed), 1);

    let second = pool
        .try_insert(DropValue {
            id: 2,
            drops: Arc::clone(&drops),
        })
        .unwrap_or_else(|_| panic!("the returned slot must be reusable"));
    assert_eq!(second.id, 2);
    drop(second);
    assert_eq!(drops.load(Ordering::Relaxed), 2);
}

#[test]
fn an_in_flight_payload_keeps_the_pool_alive() {
    let drops = Arc::new(AtomicUsize::new(0));
    let pool = PayloadPool::new(1);
    let payload = pool
        .try_insert(DropValue {
            id: 7,
            drops: Arc::clone(&drops),
        })
        .unwrap_or_else(|_| panic!("slot available"));

    drop(pool);
    assert_eq!(payload.id, 7);
    drop(payload);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}
