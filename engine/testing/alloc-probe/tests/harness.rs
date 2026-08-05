//! What the burst assertion can and cannot see, measured against a real allocator.
//!
//! These live in an integration test rather than beside the code because the
//! property under test is a property of a *binary*: the assertion is only
//! meaningful where a counting `#[global_allocator]` is installed, and an
//! integration test is the smallest unit that can install one. The crate's own
//! unit tests are the opposite control — that binary installs no counting
//! allocator, so the self-check must refuse to certify anything there.

use migo_alloc_probe::{
    Burst, CountingAllocator, assert_no_steady_state_allocation, thread_counts,
};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

#[test]
fn a_fresh_block_is_counted_as_an_allocation() {
    let before = thread_counts();
    let block: Vec<u8> = Vec::with_capacity(64);
    std::hint::black_box(&block);
    let after = thread_counts();

    assert_eq!(after.allocations - before.allocations, 1);
    assert!(after.bytes_allocated - before.bytes_allocated >= 64);
    assert_eq!(after.reallocations - before.reallocations, 0);
}

#[test]
fn growing_past_reserved_capacity_is_counted_as_an_allocation_event() {
    let mut block: Vec<u8> = Vec::with_capacity(4);
    let before = thread_counts();
    block.extend_from_slice(&[0; 4096]);
    std::hint::black_box(&block);
    let after = thread_counts();

    assert!(after.reallocations > before.reallocations);
    assert!(after.allocation_events() > before.allocation_events());
}

#[test]
fn releasing_a_block_is_not_an_allocation_event() {
    let mut blocks: Vec<Vec<u8>> = (0..8).map(|_| Vec::with_capacity(64)).collect();
    let before = thread_counts();
    while let Some(block) = blocks.pop() {
        drop(block);
    }
    let after = thread_counts();

    assert!(after.deallocations > before.deallocations);
    assert_eq!(after.allocation_events(), before.allocation_events());
}

#[test]
fn a_burst_that_stays_off_the_heap_passes() {
    let mut total = 0u64;
    assert_no_steady_state_allocation(
        Burst {
            path: "arithmetic only",
            warmup: 2,
            measured: 16,
        },
        |iteration| {
            total += iteration as u64;
            total
        },
    );
    assert_eq!(total, (0..18u64).sum::<u64>());
}

#[test]
#[should_panic(expected = "pointer motion: 16 heap allocation event(s)")]
fn a_burst_that_allocates_every_iteration_fails_and_names_the_path() {
    assert_no_steady_state_allocation(
        Burst {
            path: "pointer motion",
            warmup: 2,
            measured: 16,
        },
        |_| Vec::<u8>::with_capacity(32),
    );
}

#[test]
fn allocation_during_warmup_is_not_steady_state() {
    assert_no_steady_state_allocation(
        Burst {
            path: "lazily initialised on first use",
            warmup: 3,
            measured: 16,
        },
        |iteration| {
            if iteration < 3 {
                Some(Vec::<u8>::with_capacity(32))
            } else {
                None
            }
        },
    );
}

#[test]
#[should_panic(expected = "1 heap allocation event(s)")]
fn a_single_allocation_in_the_last_measured_iteration_still_fails() {
    assert_no_steady_state_allocation(
        Burst {
            path: "allocates once, late",
            warmup: 2,
            measured: 16,
        },
        |iteration| {
            if iteration == 17 {
                Some(Vec::<u8>::with_capacity(32))
            } else {
                None
            }
        },
    );
}

#[test]
fn a_measured_burst_reports_the_iteration_count_it_covered() {
    let panic = std::panic::catch_unwind(|| {
        assert_no_steady_state_allocation(
            Burst {
                path: "reported",
                warmup: 1,
                measured: 5,
            },
            |_| Vec::<u8>::with_capacity(8),
        );
    })
    .expect_err("an allocating burst must fail");
    let message = panic
        .downcast_ref::<String>()
        .expect("assertion messages are formatted");

    assert!(
        message.contains("over 5 measured iteration(s)"),
        "message did not report the measured span: {message}"
    );
}
