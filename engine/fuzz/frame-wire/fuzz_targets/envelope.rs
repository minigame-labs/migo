#![no_main]

//! The envelope parser against arbitrary bytes.
//!
//! The property is not "it rejects bad packets" -- the unit tests cover the
//! specific rules. It is that no input reaches a panic, an out-of-bounds read,
//! an unbounded loop or an allocation the input chose the size of. This code
//! runs on the render path, fed by content JavaScript in another process; a
//! panic there is an abort in a shipped app that content can trigger.
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
        // The checksum is recomputed over the accepted bytes to keep the
        // hashing path in the fuzzed surface rather than only the parse path.
        std::hint::black_box(frame_wire::checksum(data));
    }
});
