#![no_main]

//! The envelope parser and the ingress against arbitrary bytes.
//!
//! The property is not "it rejects bad packets" -- the unit tests cover the
//! specific rules. It is that no input reaches a panic, an out-of-bounds read,
//! an unbounded loop or an allocation the input chose the size of. This code
//! runs on the render path, fed by content JavaScript in another process; a
//! panic there is an abort in a shipped app that content can trigger.
//!
//! The ingress is fuzzed alongside the parser rather than left to unit tests.
//! Its state -- last sequence, credits, epoch, readiness -- advances across
//! calls, so the interesting inputs are *sequences* of packets, and a single
//! shared instance across one fuzz case is what exposes an arithmetic edge that
//! only appears after an accept.
//!
//! Run:  cargo +nightly fuzz run envelope
//! Corpus seeds: contracts/frame-wire/golden/*.bin

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(frame) = frame_wire::validate(data) {
        // Walking the sections is part of the surface: `sections()` indexes
        // without re-checking, on the strength of validation having run. If
        // that reasoning is ever wrong, this is where it faults.
        for section in frame.sections() {
            std::hint::black_box(section.bytes.len());
            std::hint::black_box(section.item_count);
        }
        std::hint::black_box(frame.command_stream().is_some());
        std::hint::black_box(frame.references_resources());
        std::hint::black_box(frame.launch_nonce());
        std::hint::black_box(frame.sequence());
        // The checksum is recomputed over the accepted bytes to keep the
        // hashing path in the fuzzed surface rather than only the parse path.
        std::hint::black_box(frame_wire::checksum(data));
    }

    // A nonce and generation taken from the input, so a case can address the
    // ingress correctly and reach the accept path rather than stopping at the
    // identity check. Chunking the input into packets exercises the sequence
    // and credit state machine across calls.
    let mut nonce_bytes = [0u8; 16];
    let take = data.len().min(16);
    nonce_bytes[..take].copy_from_slice(&data[..take]);
    let mut ingress = frame_wire::FrameIngress::new(
        u128::from_le_bytes(nonce_bytes),
        u64::from(data.first().copied().unwrap_or(0)),
    );
    if data.len() % 3 == 0 {
        ingress.mark_resources_ready();
    }
    let mut ingress = ingress.with_max_packet_bytes(u32::from(data.len() as u16).max(80));

    for chunk in data.chunks(96.max(data.len() / 4 + 1)).take(8) {
        let (outcome, frame) = ingress.submit(chunk);
        std::hint::black_box(outcome.remaining_credits);
        std::hint::black_box(outcome.wire_error_code);
        if frame.is_some() {
            ingress.complete();
        }
    }
    // Timeline moves, including refused ones, on a live instance.
    let _ = ingress.set_surface_generation(u64::from(data.len() as u32));
    let _ = ingress.set_resource_epoch(u64::from(data.last().copied().unwrap_or(0)));
    std::hint::black_box(ingress.remaining_credits());
});
