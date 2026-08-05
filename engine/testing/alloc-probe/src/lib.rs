//! Counting allocator and burst assertion for Section 7.3's steady-state
//! allocation gate.
//!
//! Section 7.3 of the four-platform delivery design requires, for every steady hot
//! path, "a test that counts actual allocations during a burst of events and fails
//! when the count is non-zero". Greping the sources for `with_capacity` asserts that
//! the code was *written* not to allocate, which cannot observe an allocation and so
//! cannot fail when one appears. This crate is what observes it.
//!
//! **Why a crate of its own.** `#[global_allocator]` is unique per binary. A counting
//! allocator declared in a shipped library would follow that library into every
//! cdylib Migo ships. Living here, reachable only through `[dev-dependencies]`, makes
//! "production code cannot use this" a fact cargo enforces rather than a comment
//! asking politely.
//!
//! **Why the counters are per thread.** `cargo test` runs tests concurrently in one
//! process with one global allocator, so process-wide counters would attribute every
//! other test's allocations to the burst under measurement.
//!
//! Usage, from the crate whose hot path is being gated:
//!
//! ```ignore
//! #[cfg(test)]
//! #[global_allocator]
//! static COUNTING_ALLOCATOR: migo_alloc_probe::CountingAllocator =
//!     migo_alloc_probe::CountingAllocator::system();
//! ```

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    hint::black_box,
};

struct Counters {
    allocations: Cell<u64>,
    reallocations: Cell<u64>,
    deallocations: Cell<u64>,
    bytes_allocated: Cell<u64>,
}

thread_local! {
    // `const` initialisation with a `Drop`-free payload is load-bearing, not a
    // micro-optimisation: a lazily initialised thread-local allocates on first
    // touch, and this one is touched from inside the allocator.
    static COUNTERS: Counters = const {
        Counters {
            allocations: Cell::new(0),
            reallocations: Cell::new(0),
            deallocations: Cell::new(0),
            bytes_allocated: Cell::new(0),
        }
    };
}

/// Allocation activity observed on one thread.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AllocationCounts {
    pub allocations: u64,
    pub reallocations: u64,
    pub deallocations: u64,
    pub bytes_allocated: u64,
}

impl AllocationCounts {
    /// Calls that reached the heap for memory: a fresh block or a resize.
    ///
    /// A resize counts. A container that outgrows its reserved capacity never calls
    /// `alloc` — it calls `realloc` — so a gate blind to resizes would miss exactly
    /// the regression it exists to catch, a `with_capacity` quietly becoming a `new`.
    #[must_use]
    pub const fn allocation_events(&self) -> u64 {
        self.allocations + self.reallocations
    }

    #[must_use]
    fn since(self, earlier: Self) -> Self {
        Self {
            allocations: self.allocations - earlier.allocations,
            reallocations: self.reallocations - earlier.reallocations,
            deallocations: self.deallocations - earlier.deallocations,
            bytes_allocated: self.bytes_allocated - earlier.bytes_allocated,
        }
    }
}

/// This thread's totals since it started.
///
/// Zero everywhere means either a quiet thread or a binary with no counting
/// allocator installed. The two are indistinguishable from here, which is why
/// [`assert_no_steady_state_allocation`] proves installation before trusting a zero.
#[must_use]
pub fn thread_counts() -> AllocationCounts {
    COUNTERS
        .try_with(|counters| AllocationCounts {
            allocations: counters.allocations.get(),
            reallocations: counters.reallocations.get(),
            deallocations: counters.deallocations.get(),
            bytes_allocated: counters.bytes_allocated.get(),
        })
        .unwrap_or_default()
}

// `wrapping_add` rather than `+`: a panic raised inside `alloc` cannot unwind
// sensibly, and `try_with` rather than `with` for the same reason during thread
// teardown. Both trade an impossible overflow and an untouchable teardown
// allocation for the guarantee that the allocator itself never panics.
fn record_allocation(bytes: usize) {
    let _ = COUNTERS.try_with(|counters| {
        counters
            .allocations
            .set(counters.allocations.get().wrapping_add(1));
        counters
            .bytes_allocated
            .set(counters.bytes_allocated.get().wrapping_add(bytes as u64));
    });
}

fn record_reallocation(new_bytes: usize) {
    let _ = COUNTERS.try_with(|counters| {
        counters
            .reallocations
            .set(counters.reallocations.get().wrapping_add(1));
        counters.bytes_allocated.set(
            counters
                .bytes_allocated
                .get()
                .wrapping_add(new_bytes as u64),
        );
    });
}

fn record_deallocation() {
    let _ = COUNTERS.try_with(|counters| {
        counters
            .deallocations
            .set(counters.deallocations.get().wrapping_add(1));
    });
}

/// A [`GlobalAlloc`] that counts what passes through it, per thread.
///
/// Every method delegates; the counting is the only added behaviour, and it never
/// allocates, so installing this cannot change what the code under measurement does.
pub struct CountingAllocator<A = System> {
    inner: A,
}

impl CountingAllocator<System> {
    #[must_use]
    pub const fn system() -> Self {
        Self { inner: System }
    }
}

// SAFETY: every method forwards its arguments unchanged to `inner`, which upholds
// `GlobalAlloc`'s contract; the counting added around each call touches only
// `Cell<u64>` thread-locals and allocates nothing.
unsafe impl<A: GlobalAlloc> GlobalAlloc for CountingAllocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation(layout.size());
        unsafe { self.inner.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation(layout.size());
        unsafe { self.inner.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_reallocation(new_size);
        unsafe { self.inner.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record_deallocation();
        unsafe { self.inner.dealloc(ptr, layout) }
    }
}

/// One measured burst.
///
/// A struct rather than three positional arguments because `warmup` and `measured`
/// are both counts, and transposing them at a call site would silently move the
/// measurement window onto the startup path it exists to exclude.
pub struct Burst<'a> {
    /// The hot path under measurement, quoted verbatim in the failure.
    pub path: &'a str,
    /// Iterations run before measurement begins. One-time lazy initialisation — a
    /// thread parker, a pool's first fill, a channel's first notification — is not
    /// steady state, and counting it would make every gate fail for the wrong reason.
    pub warmup: usize,
    /// Iterations measured. The burst body receives a continuous iteration index, so
    /// the measured span is `warmup..warmup + measured`.
    pub measured: usize,
}

/// Run a burst of events and fail if any of the measured iterations reached the heap.
///
/// The body's return value is passed through [`black_box`] so an optimised build
/// cannot delete the work being measured.
#[track_caller]
pub fn assert_no_steady_state_allocation<T>(burst: Burst<'_>, mut body: impl FnMut(usize) -> T) {
    assert!(
        burst.warmup > 0,
        "{}: warmup iterations must be non-zero, or the measurement covers first-use \
         initialisation instead of steady state",
        burst.path
    );
    assert!(
        burst.measured > 0,
        "{}: measured iterations must be non-zero, or the assertion proves nothing",
        burst.path
    );

    assert_counting_allocator_is_installed(burst.path);

    for iteration in 0..burst.warmup {
        black_box(body(iteration));
    }

    let before = thread_counts();
    for iteration in burst.warmup..burst.warmup + burst.measured {
        black_box(body(iteration));
    }
    let delta = thread_counts().since(before);

    assert!(
        delta.allocation_events() == 0,
        "{path}: {events} heap allocation event(s) over {measured} measured iteration(s) \
         ({allocations} fresh, {reallocations} resize, {bytes} bytes; {deallocations} release(s)). \
         Section 7.3 requires zero steady-state allocation on this path.",
        path = burst.path,
        events = delta.allocation_events(),
        measured = burst.measured,
        allocations = delta.allocations,
        reallocations = delta.reallocations,
        bytes = delta.bytes_allocated,
        deallocations = delta.deallocations,
    );
}

/// Refuse to certify a burst in a binary that cannot observe allocations.
///
/// Without this, deleting one `#[global_allocator]` line turns every allocation gate
/// in that binary into an unconditional pass — a silent, permanent, green failure.
/// So each burst first allocates something it knows about and insists on seeing it.
#[track_caller]
fn assert_counting_allocator_is_installed(path: &str) {
    let before = thread_counts();
    let probe: Vec<u8> = Vec::with_capacity(64);
    black_box(&probe);
    drop(black_box(probe));
    let observed = thread_counts().since(before);

    assert!(
        observed.allocation_events() > 0,
        "{path}: the counting allocator did not observe a known allocation, so a zero \
         count on this path would prove nothing. Declare it in this binary:\n  \
         #[cfg(test)]\n  #[global_allocator]\n  static COUNTING_ALLOCATOR: \
         migo_alloc_probe::CountingAllocator = migo_alloc_probe::CountingAllocator::system();"
    );
}

#[cfg(test)]
mod tests {
    use super::{Burst, assert_no_steady_state_allocation};

    // This binary deliberately installs no counting allocator, which makes it the
    // negative control for installation. `tests/harness.rs` is the positive one.

    #[test]
    #[should_panic(expected = "the counting allocator did not observe a known allocation")]
    fn a_burst_refuses_to_certify_a_binary_without_the_allocator() {
        assert_no_steady_state_allocation(
            Burst {
                path: "uninstrumented binary",
                warmup: 1,
                measured: 1,
            },
            |_| (),
        );
    }

    #[test]
    #[should_panic(expected = "measured iterations must be non-zero")]
    fn a_burst_measuring_nothing_is_rejected() {
        assert_no_steady_state_allocation(
            Burst {
                path: "vacuous",
                warmup: 1,
                measured: 0,
            },
            |_| (),
        );
    }

    #[test]
    #[should_panic(expected = "warmup iterations must be non-zero")]
    fn a_burst_without_warmup_is_rejected() {
        assert_no_steady_state_allocation(
            Burst {
                path: "cold",
                warmup: 0,
                measured: 1,
            },
            |_| (),
        );
    }
}
