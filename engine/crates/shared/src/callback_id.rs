//! Host callback identifiers: issued once, never reused, never wrapped.
//!
//! A restart replaces the JavaScript isolate while asynchronous work issued by
//! the retired one is still in flight. If an identifier could be reused, a
//! result belonging to the retired runtime would match a registration made by
//! its replacement and be delivered into the wrong isolate. Uniqueness for the
//! whole `Host` lifetime is what makes that impossible, so this allocator has
//! no reset, release or free — the absence is the mechanism.
//!
//! Exhaustion is therefore permanent rather than a reason to wrap: at
//! `i32::MAX` identifiers the only safe answer is to stop issuing them. That
//! bound is `i32` and not `u32` because these cross the JNI and C boundaries as
//! signed 32-bit values.

use std::{
    error::Error,
    fmt,
    sync::atomic::{AtomicU32, Ordering::Relaxed},
};

/// The identifier space is spent. Permanent for the remaining `Host` lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallbackIdExhausted;

impl fmt::Display for CallbackIdExhausted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("host callback id space exhausted")
    }
}

impl Error for CallbackIdExhausted {}

/// Issues positive `i32` callback identifiers, once each.
#[derive(Debug, Default)]
pub struct CallbackIdAllocator {
    last_issued: AtomicU32,
}

impl CallbackIdAllocator {
    /// The next identifier, or `Err` once the space is spent.
    ///
    /// `Relaxed` is sufficient and is not a shortcut: a read-modify-write is
    /// atomic whatever its ordering, so two callers cannot observe the same
    /// previous value and no identifier can be issued twice. Ordering would
    /// only matter if this call published other memory, and it does not — the
    /// caller registers the identifier under its own synchronisation.
    pub fn allocate(&self) -> Result<i32, CallbackIdExhausted> {
        let previous = self
            .last_issued
            .fetch_update(Relaxed, Relaxed, |last| {
                (last < i32::MAX as u32).then_some(last + 1)
            })
            .map_err(|_| CallbackIdExhausted)?;
        // `previous < i32::MAX` held for the update to succeed, so the sum is
        // at most `i32::MAX` and the cast cannot lose or sign-flip.
        Ok((previous + 1) as i32)
    }

    /// An allocator one identifier short of exhaustion.
    ///
    /// Test-only, and deliberately not a public `with_last_issued`: a caller
    /// able to choose the starting point is a caller able to reissue an
    /// identifier, which is the single thing this type exists to prevent.
    #[cfg(test)]
    fn nearly_exhausted() -> Self {
        Self {
            last_issued: AtomicU32::new(i32::MAX as u32 - 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc, thread};

    use super::{CallbackIdAllocator, CallbackIdExhausted};

    #[test]
    fn starts_at_one_and_never_reuses_ids() {
        let ids = CallbackIdAllocator::default();
        // Zero is reserved: several boundaries treat it as "no callback", so an
        // allocator that started there would hand out an identifier meaning
        // "absent".
        assert_eq!(ids.allocate(), Ok(1));
        assert_eq!(ids.allocate(), Ok(2));
        assert_eq!(ids.allocate(), Ok(3));
    }

    #[test]
    fn arc_clones_allocate_one_global_sequence() {
        let ids = Arc::new(CallbackIdAllocator::default());
        let shared = Arc::clone(&ids);

        assert_eq!(ids.allocate(), Ok(1));
        assert_eq!(shared.allocate(), Ok(2));
        assert_eq!(ids.allocate(), Ok(3));
    }

    #[test]
    fn threads_never_duplicate_an_id() {
        const THREADS: usize = 8;
        const PER_THREAD: usize = 512;

        let ids = Arc::new(CallbackIdAllocator::default());
        let issued: HashSet<i32> = thread::scope(|scope| {
            let handles: Vec<_> = (0..THREADS)
                .map(|_| {
                    let ids = Arc::clone(&ids);
                    scope.spawn(move || {
                        (0..PER_THREAD)
                            .map(|_| ids.allocate().expect("space is not near its bound"))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|handle| handle.join().expect("allocating thread"))
                .collect()
        });

        assert_eq!(
            issued.len(),
            THREADS * PER_THREAD,
            "an id was issued to more than one caller"
        );
        assert!(issued.iter().all(|id| *id >= 1));
    }

    #[test]
    fn maximum_is_issued_once_then_exhaustion_is_permanent() {
        let ids = CallbackIdAllocator::nearly_exhausted();

        assert_eq!(ids.allocate().unwrap(), i32::MAX);
        assert_eq!(ids.allocate(), Err(CallbackIdExhausted));
        // Asked twice, because "exhausted" that recovers on the next call is a
        // wrap wearing an error's name.
        assert_eq!(ids.allocate(), Err(CallbackIdExhausted));
    }

    #[test]
    fn exhaustion_reports_a_stable_message() {
        assert_eq!(
            CallbackIdExhausted.to_string(),
            "host callback id space exhausted"
        );
    }
}
