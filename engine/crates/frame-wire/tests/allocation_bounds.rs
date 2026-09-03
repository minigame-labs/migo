//! The parser and the ingress allocate nothing, asserted rather than described.
//!
//! "Validate before allocating" is the first rule in this crate's threat model,
//! and until now it was a comment. A comment cannot notice a `Vec` added inside
//! a rejection path, or a `format!` in an error message, or a collect that
//! looked harmless because the count was small in the test.
//!
//! So a counting allocator is installed for this test binary and armed around
//! the measured calls only. The window is armed *after* every input has been
//! built, because the inputs themselves allocate and a probe that measures its
//! own setup reports a number nobody can act on.
//!
//! This file holds exactly one test on purpose. The counters are global, Rust
//! runs tests in the same binary on several threads, and a second test would
//! make this one's number depend on scheduling.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use frame_wire::{
    FrameIngress, HEADER_BYTES, SECTION_KIND_COMMAND_STREAM, SECTION_KIND_DAMAGE,
    SECTION_KIND_INLINE_DATA, SECTION_KIND_RESOURCE_REFERENCES, builder::WireFrameBuilder,
    stamp_checksum, validate,
};

static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// A deterministic byte generator. A fuzzer belongs in the fuzz target; what
/// this needs is a fixed sweep that runs on every `cargo test`.
fn xorshift(state: &mut u64) -> u8 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state & 0xFF) as u8
}

const NONCE: u128 = 0xFEDC_BA98_7654_3210_0123_4567_89AB_CDEF;
const GENERATION: u64 = 3;

fn inputs() -> Vec<Vec<u8>> {
    let mut inputs = Vec::new();

    let stream = [0u8; 32];
    let inline = [9u8; 21];
    let refs = [1u8; 12];
    let damage = [2u8; 16];

    let mut valid = WireFrameBuilder::new();
    valid.launch_nonce = NONCE;
    valid.runtime_generation = GENERATION;
    inputs.push(
        valid
            .section(SECTION_KIND_COMMAND_STREAM, 8, &stream)
            .section(SECTION_KIND_INLINE_DATA, 21, &inline)
            .section(SECTION_KIND_RESOURCE_REFERENCES, 3, &refs)
            .section(SECTION_KIND_DAMAGE, 1, &damage)
            .build(),
    );

    // Single-byte corruption at every header and table offset, restamped and
    // not, so both the rejection paths and the checksum path are measured.
    let template = inputs[0].clone();
    let table_end = HEADER_BYTES as usize + 4 * 16;
    for offset in 0..table_end.min(template.len()) {
        for value in [0u8, 0x7F, 0xFF] {
            let mut bytes = template.clone();
            bytes[offset] = value;
            inputs.push(bytes.clone());
            stamp_checksum(&mut bytes);
            inputs.push(bytes);
        }
    }

    // Arbitrary bytes at a spread of lengths, including lengths around the
    // header size where the truncation branches live.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for length in [0usize, 1, 17, 63, 79, 80, 81, 96, 200, 4096] {
        let mut bytes = vec![0u8; length];
        for byte in bytes.iter_mut() {
            *byte = xorshift(&mut state);
        }
        inputs.push(bytes);
    }

    inputs
}

#[test]
fn neither_the_parser_nor_the_ingress_allocates_for_any_input() {
    let inputs = inputs();
    let mut ingress = FrameIngress::new(NONCE, GENERATION);

    ARMED.store(true, Ordering::SeqCst);
    let mut accepted = 0u64;
    let mut walked = 0usize;
    for bytes in &inputs {
        if let Ok(frame) = validate(bytes) {
            for section in frame.sections() {
                walked += section.bytes.len();
            }
            walked += frame
                .command_stream()
                .map_or(0, |section| section.bytes.len());
            walked += usize::from(frame.references_resources());
        }
        let (outcome, frame) = ingress.submit(bytes);
        if frame.is_some() {
            accepted += 1;
            ingress.complete();
        }
        walked += outcome.wire_error_code as usize;
    }
    ARMED.store(false, Ordering::SeqCst);

    let allocations = ALLOCATIONS.load(Ordering::SeqCst);
    let bytes_allocated = ALLOCATED_BYTES.load(Ordering::SeqCst);
    assert_eq!(
        allocations,
        0,
        "validate/submit allocated {allocations} times ({bytes_allocated} bytes) across \
         {} inputs; the parser's contract is that content cannot make it allocate",
        inputs.len()
    );

    // The measured loop has to have done something, or zero allocations is the
    // number a loop that never ran also reports.
    assert!(accepted >= 1, "no input was ever accepted");
    assert!(walked > 0, "no section was ever walked");
    assert!(
        inputs.len() > 100,
        "the sweep is too small to mean anything"
    );
}
