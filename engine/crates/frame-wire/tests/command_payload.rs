//! The envelope and the payload, together, with no JavaScript engine in sight.
//!
//! This is the seam the whole cross-process lane rests on: bytes produced by
//! content JavaScript in another process arrive here, and this process decides
//! whether to draw them. Two layers, deliberately separate --
//!
//!   * [`frame_wire::validate`] checks the envelope: header, section table,
//!     bounds, checksum. It knows nothing about opcodes.
//!   * [`frame_wire::stream::validate_stream`] checks the command payload.
//!     It knows nothing about packets.
//!
//! -- so that adding an opcode cannot break envelope correctness, and neither
//! layer has to be re-reviewed when the other changes.
//!
//! What makes these cases worth having beyond the two suites that already test
//! each layer: they run in a crate whose dependency list is `crc32fast`. If the
//! payload validator were still reachable only through the JavaScript runtime,
//! this file could not exist, and `MigoApplePerformancePlus` could not claim a
//! dependency closure without a JavaScript engine.

use frame_wire::{
    SECTION_KIND_COMMAND_STREAM, SECTION_KIND_INLINE_DATA,
    builder::WireFrameBuilder,
    gl::{OP_CLEAR, OP_CLEAR_COLOR},
    stream::{MAGIC, STREAM_VERSION, StreamError, pack_header, validate_stream},
    validate,
};

/// A real, minimal WebGL frame: set the clear colour, then clear.
///
/// `word_count` in a record header counts the header itself, so
/// `OP_CLEAR_COLOR`'s 6 is one header plus a canvas id and four floats, and
/// `OP_CLEAR`'s 3 is one header plus a canvas id and the bit field. Getting
/// this wrong is what a fixture written from the opcode name alone does, and
/// the validator caught it -- which is the fixture doing its job before the
/// test does.
const CANVAS_ID: u32 = 1;

fn clear_frame() -> Vec<u32> {
    let mut words = vec![MAGIC, STREAM_VERSION];
    words.push(pack_header(OP_CLEAR_COLOR, 6));
    words.push(CANVAS_ID);
    words.extend_from_slice(&[
        0f32.to_bits(),
        0f32.to_bits(),
        0f32.to_bits(),
        1f32.to_bits(),
    ]);
    words.push(pack_header(OP_CLEAR, 3));
    words.push(CANVAS_ID);
    words.push(0x0000_4000); // GL_COLOR_BUFFER_BIT
    words
}

fn to_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn from_bytes(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[test]
fn a_packet_carrying_a_real_command_stream_passes_both_layers() {
    let words = clear_frame();
    let payload = to_bytes(&words);
    let packet = WireFrameBuilder::new()
        .section(SECTION_KIND_COMMAND_STREAM, words.len() as u32, &payload)
        .build();

    let frame = validate(&packet).expect("envelope");
    let section = frame.command_stream().expect("command stream section");

    let recovered = from_bytes(section.bytes);
    assert_eq!(
        recovered, words,
        "the payload survived the envelope unchanged"
    );
    assert!(
        validate_stream(&recovered, recovered.len() as u32).is_ok(),
        "payload must validate on its own",
    );
}

#[test]
fn a_valid_envelope_does_not_vouch_for_the_payload() {
    // The separation, stated as a test. A packet can be perfectly formed and
    // still carry commands that must not be executed -- which is why the
    // consumer runs both validators and not just the one that answered first.
    let mut words = clear_frame();
    words[2] = pack_header(0xFFF, 3); // an opcode that does not exist
    let payload = to_bytes(&words);
    let packet = WireFrameBuilder::new()
        .section(SECTION_KIND_COMMAND_STREAM, words.len() as u32, &payload)
        .build();

    let frame = validate(&packet).expect("the envelope is well formed");
    let recovered = from_bytes(frame.command_stream().unwrap().bytes);
    let rejected = validate_stream(&recovered, recovered.len() as u32)
        .expect_err("the payload must be rejected");
    assert!(
        matches!(rejected, StreamError::UnknownOpcode(0xFFF)),
        "the rejection should name the opcode, not just fail: {rejected:?}",
    );
    assert_ne!(rejected.code(), 0, "a rejection must not read as success");
}

#[test]
fn a_corrupted_payload_is_caught_by_the_envelope_before_the_decoder_sees_it() {
    // Order matters on the real path: the checksum runs before anything walks
    // the command stream, so a flipped bit costs one CRC rather than a walk
    // over words that were never what the producer sent.
    let words = clear_frame();
    let payload = to_bytes(&words);
    let mut packet = WireFrameBuilder::new()
        .section(SECTION_KIND_COMMAND_STREAM, words.len() as u32, &payload)
        .build();

    let last = packet.len() - 1;
    packet[last] ^= 0x01;
    assert!(validate(&packet).is_err(), "the envelope must catch this");
}

#[test]
fn the_payload_offset_stays_word_aligned_behind_a_ragged_inline_section() {
    // A `u32` view of the command stream is only sound if its offset is a
    // multiple of four, and section padding is what guarantees that. An inline
    // blob of a deliberately awkward length is what would break it.
    let words = clear_frame();
    let payload = to_bytes(&words);
    let ragged = [0xABu8; 13];
    let packet = WireFrameBuilder::new()
        .section(SECTION_KIND_INLINE_DATA, 13, &ragged)
        .section(SECTION_KIND_COMMAND_STREAM, words.len() as u32, &payload)
        .build();

    let frame = validate(&packet).expect("envelope");
    let section = frame.command_stream().expect("command stream");
    let offset = section.bytes.as_ptr() as usize - packet.as_ptr() as usize;
    assert_eq!(
        offset % 8,
        0,
        "sections are 8-byte aligned within the packet"
    );
    assert_eq!(from_bytes(section.bytes), words);
}

/// The dependency claim, asserted rather than described.
///
/// The payload validator has to be usable from a crate that links no JavaScript
/// engine, because that is the property `MigoApplePerformancePlus` is built on.
/// This file compiles inside `migo-frame-wire`, whose only dependency is a CRC
/// implementation -- so the fact that these calls resolve at all is the check.
/// Stating it as a test keeps it from being quietly undone by a convenience
/// dependency added later.
#[test]
fn the_payload_validator_is_reachable_without_a_javascript_engine() {
    let words = clear_frame();
    assert!(validate_stream(&words, words.len() as u32).is_ok());
    assert_eq!(
        validate_stream(&[MAGIC], 1).unwrap_err(),
        StreamError::TooShort
    );
}
