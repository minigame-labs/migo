//! Every rejection path, one case each, plus the two properties that hold for
//! all inputs.
//!
//! The cases mutate a packet that is known good, so each one isolates a single
//! violation. A test that builds a bad packet from scratch tends to be wrong in
//! several ways at once and then passes for the wrong reason.

use frame_wire::{
    FLAG_PRESENT, HEADER_BYTES, MAX_SECTIONS, SECTION_ENTRY_BYTES, SECTION_KIND_COMMAND_STREAM,
    SECTION_KIND_DAMAGE, SECTION_KIND_INLINE_DATA, WIRE_MAGIC, WIRE_VERSION, WireError,
    builder::WireFrameBuilder, stamp_checksum, validate,
};

const OFF_MAGIC: usize = 0;
const OFF_WIRE_VERSION: usize = 4;
const OFF_HEADER_BYTES: usize = 8;
const OFF_TOTAL_BYTES: usize = 12;
const OFF_FLAGS: usize = 48;
const OFF_SECTION_COUNT: usize = 52;
const OFF_CHECKSUM: usize = 56;
const OFF_RESERVED0: usize = 60;

/// A minimal, valid packet: one command stream of four words.
fn good() -> Vec<u8> {
    let stream = [0u8; 16];
    WireFrameBuilder::new()
        .section(SECTION_KIND_COMMAND_STREAM, 4, &stream)
        .build()
}

fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// Corrupt a field and restamp, so the case under test is the only violation.
/// Without restamping every mutation would be reported as a checksum failure --
/// technically a rejection, and useless as evidence that the specific rule works.
fn mutate(mutation: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut bytes = good();
    mutation(&mut bytes);
    stamp_checksum(&mut bytes);
    bytes
}

#[test]
fn a_well_formed_packet_round_trips() {
    let bytes = good();
    let frame = validate(&bytes).expect("the builder must produce a valid packet");

    assert_eq!(frame.total_bytes(), bytes.len());
    assert_eq!(frame.section_count(), 1);
    assert!(frame.presents());
    assert_eq!(frame.sequence(), 1);

    let stream = frame.command_stream().expect("command stream section");
    assert_eq!(stream.kind, SECTION_KIND_COMMAND_STREAM);
    assert_eq!(stream.bytes.len(), 16);
    assert_eq!(stream.item_count, 4);
}

#[test]
fn sections_come_back_in_the_order_they_were_written() {
    let stream = [0u8; 8];
    let inline = [7u8; 12];
    let damage = [0u8; 16];
    let bytes = WireFrameBuilder::new()
        .section(SECTION_KIND_COMMAND_STREAM, 2, &stream)
        .section(SECTION_KIND_INLINE_DATA, 12, &inline)
        .section(SECTION_KIND_DAMAGE, 1, &damage)
        .build();

    let frame = validate(&bytes).expect("three sections is valid");
    let kinds: Vec<u32> = frame.sections().map(|section| section.kind).collect();
    assert_eq!(
        kinds,
        vec![
            SECTION_KIND_COMMAND_STREAM,
            SECTION_KIND_INLINE_DATA,
            SECTION_KIND_DAMAGE
        ]
    );
    let inline_section = frame
        .sections()
        .find(|section| section.kind == SECTION_KIND_INLINE_DATA)
        .expect("inline section");
    assert_eq!(inline_section.bytes, &inline[..]);
}

#[test]
fn a_truncated_packet_is_rejected_before_any_field_is_read() {
    let bytes = good();
    for length in 0..HEADER_BYTES as usize {
        assert_eq!(
            validate(&bytes[..length]).unwrap_err(),
            WireError::ShorterThanHeader,
            "a {length}-byte packet must not reach field parsing",
        );
    }
}

#[test]
fn each_header_rule_has_its_own_rejection() {
    let cases: Vec<(&str, Vec<u8>, WireError)> = vec![
        (
            "magic",
            mutate(|b| put_u32(b, OFF_MAGIC, WIRE_MAGIC ^ 1)),
            WireError::BadMagic,
        ),
        (
            "version",
            mutate(|b| put_u32(b, OFF_WIRE_VERSION, WIRE_VERSION + 1)),
            WireError::UnsupportedVersion,
        ),
        (
            "header_bytes",
            mutate(|b| put_u32(b, OFF_HEADER_BYTES, HEADER_BYTES + 8)),
            WireError::BadHeaderBytes,
        ),
        (
            "reserved",
            mutate(|b| put_u32(b, OFF_RESERVED0, 1)),
            WireError::ReservedNotZero,
        ),
        (
            "total_bytes below the header",
            mutate(|b| put_u32(b, OFF_TOTAL_BYTES, 8)),
            WireError::BadTotalBytes,
        ),
        (
            "total_bytes above the cap",
            mutate(|b| put_u32(b, OFF_TOTAL_BYTES, u32::MAX)),
            WireError::BadTotalBytes,
        ),
        (
            "total_bytes disagrees with the delivered length",
            mutate(|b| put_u32(b, OFF_TOTAL_BYTES, HEADER_BYTES + 8)),
            WireError::LengthMismatch,
        ),
        (
            "unknown flag",
            mutate(|b| put_u32(b, OFF_FLAGS, FLAG_PRESENT | 0x8000_0000)),
            WireError::UnknownFlags,
        ),
        (
            "section_count above the cap",
            mutate(|b| put_u32(b, OFF_SECTION_COUNT, MAX_SECTIONS + 1)),
            WireError::TooManySections,
        ),
        (
            "section table does not fit",
            mutate(|b| put_u32(b, OFF_SECTION_COUNT, MAX_SECTIONS)),
            WireError::SectionTableOutOfBounds,
        ),
    ];

    for (name, bytes, expected) in cases {
        assert_eq!(
            validate(&bytes).unwrap_err(),
            expected,
            "case '{name}' produced the wrong rejection",
        );
    }
}

#[test]
fn each_section_rule_has_its_own_rejection() {
    let entry = HEADER_BYTES as usize;

    let unknown_required = mutate(|b| put_u32(b, entry, 0x0000_0042));
    assert_eq!(
        validate(&unknown_required).unwrap_err(),
        WireError::UnknownRequiredSection,
    );

    // Advisory kinds go the other way: a reader that does not know one skips
    // it. This is the forward-compatibility half of the same rule, and testing
    // only the rejection would leave the skip unproven.
    let stream = [0u8; 8];
    let future = [0u8; 8];
    let with_future_advisory = WireFrameBuilder::new()
        .section(SECTION_KIND_COMMAND_STREAM, 2, &stream)
        .section(0x8000_00FF, 1, &future)
        .build();
    let frame = validate(&with_future_advisory).expect("an unknown advisory kind must be accepted");
    assert_eq!(frame.section_count(), 2);

    let misaligned = mutate(|b| {
        let offset = u32::from_le_bytes([b[entry + 4], b[entry + 5], b[entry + 6], b[entry + 7]]);
        put_u32(b, entry + 4, offset + 4);
    });
    assert_eq!(
        validate(&misaligned).unwrap_err(),
        WireError::SectionMisaligned,
    );

    let past_the_end = mutate(|b| put_u32(b, entry + 8, 4096));
    assert_eq!(
        validate(&past_the_end).unwrap_err(),
        WireError::SectionOutOfBounds,
    );

    // offset + length must not be allowed to wrap. Without checked arithmetic
    // this pair computes an end of 8, which is inside the packet.
    let overflowing = mutate(|b| {
        put_u32(b, entry + 4, 0xFFFF_FFF8);
        put_u32(b, entry + 8, 16);
    });
    assert!(matches!(
        validate(&overflowing).unwrap_err(),
        WireError::SectionOutOfBounds | WireError::SectionsOverlapOrUnordered,
    ));

    let not_word_aligned = mutate(|b| put_u32(b, entry + 8, 13));
    assert_eq!(
        validate(&not_word_aligned).unwrap_err(),
        WireError::CommandStreamNotWordAligned,
    );

    let overclaimed_items = mutate(|b| put_u32(b, entry + 12, 1_000_000));
    assert_eq!(
        validate(&overclaimed_items).unwrap_err(),
        WireError::ItemCountExceedsSection,
    );
}

#[test]
fn duplicate_and_unordered_sections_are_rejected() {
    let a = [0u8; 8];
    let b = [0u8; 8];

    let duplicated = WireFrameBuilder::new()
        .section(SECTION_KIND_COMMAND_STREAM, 2, &a)
        .section(SECTION_KIND_COMMAND_STREAM, 2, &b)
        .build();
    assert_eq!(
        validate(&duplicated).unwrap_err(),
        WireError::DuplicateSection,
    );

    // Swap the two offsets so the table describes descending, overlapping
    // ranges. The bytes are otherwise untouched.
    let mut unordered = WireFrameBuilder::new()
        .section(SECTION_KIND_COMMAND_STREAM, 2, &a)
        .section(SECTION_KIND_INLINE_DATA, 8, &b)
        .build();
    let first = HEADER_BYTES as usize + 4;
    let second = HEADER_BYTES as usize + SECTION_ENTRY_BYTES as usize + 4;
    let first_offset = u32::from_le_bytes(unordered[first..first + 4].try_into().unwrap());
    let second_offset = u32::from_le_bytes(unordered[second..second + 4].try_into().unwrap());
    put_u32(&mut unordered, first, second_offset);
    put_u32(&mut unordered, second, first_offset);
    stamp_checksum(&mut unordered);
    assert_eq!(
        validate(&unordered).unwrap_err(),
        WireError::SectionsOverlapOrUnordered,
    );
}

#[test]
fn a_packet_with_no_command_stream_is_rejected() {
    let damage = [0u8; 8];
    let bytes = WireFrameBuilder::new()
        .section(SECTION_KIND_DAMAGE, 1, &damage)
        .build();
    assert_eq!(
        validate(&bytes).unwrap_err(),
        WireError::MissingCommandStream,
    );
}

#[test]
fn the_checksum_covers_the_header_not_just_the_payload() {
    // A payload flip is the easy case. The one that matters is a header field
    // that still parses: flipping frame_id leaves a structurally perfect packet,
    // and only a checksum over the header notices.
    let mut payload_flipped = good();
    let last = payload_flipped.len() - 1;
    payload_flipped[last] ^= 0xFF;
    assert_eq!(
        validate(&payload_flipped).unwrap_err(),
        WireError::ChecksumMismatch,
    );

    let mut header_flipped = good();
    put_u32(&mut header_flipped, 44, 0xDEAD_BEEF); // frame_id
    assert_eq!(
        validate(&header_flipped).unwrap_err(),
        WireError::ChecksumMismatch,
    );

    // And the checksum field is excluded from its own input, or stamping it
    // would never converge.
    let mut restamped = good();
    put_u32(&mut restamped, OFF_CHECKSUM, 0);
    stamp_checksum(&mut restamped);
    assert_eq!(restamped, good());
}

/// The property, not a case: nothing the producer can send makes this panic.
///
/// A frame arrives from another process on the render path. A panic there is
/// not a caught error -- it is an abort in a shipped app, reachable from
/// content JavaScript.
#[test]
fn no_input_can_panic_the_parser() {
    let good = good();

    // Every single-byte value at every offset in a valid packet.
    for index in 0..good.len() {
        for value in [0x00u8, 0x01, 0x7F, 0x80, 0xFE, 0xFF] {
            let mut bytes = good.clone();
            bytes[index] = value;
            let _ = validate(&bytes);
        }
    }

    // Every truncation.
    for length in 0..=good.len() {
        let _ = validate(&good[..length]);
    }

    // Structured garbage: a valid header with hostile counts and offsets.
    for count in [0u32, 1, 2, MAX_SECTIONS, MAX_SECTIONS + 1, u32::MAX] {
        for offset in [0u32, 1, 7, 8, u32::MAX - 7, u32::MAX] {
            for length in [0u32, 1, 4, u32::MAX] {
                let mut bytes = good.clone();
                put_u32(&mut bytes, OFF_SECTION_COUNT, count);
                if bytes.len() >= HEADER_BYTES as usize + 16 {
                    put_u32(&mut bytes, HEADER_BYTES as usize + 4, offset);
                    put_u32(&mut bytes, HEADER_BYTES as usize + 8, length);
                }
                stamp_checksum(&mut bytes);
                let _ = validate(&bytes);
            }
        }
    }

    // Pure noise of every length up to a couple of headers, deterministic so a
    // failure reproduces.
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    for length in 0..160usize {
        let mut bytes = vec![0u8; length];
        for byte in bytes.iter_mut() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *byte = (state >> 24) as u8;
        }
        let _ = validate(&bytes);
    }
}

/// The other property: a validated frame's section slices are always in range.
///
/// `sections()` indexes without re-checking, on the strength of validation
/// having run. That is only sound if validation actually established it, so it
/// gets asserted rather than assumed.
#[test]
fn every_accepted_frame_has_in_range_sections() {
    let stream = [1u8; 32];
    let inline = [2u8; 5];
    let timing = [3u8; 24];
    let bytes = WireFrameBuilder::new()
        .section(SECTION_KIND_COMMAND_STREAM, 8, &stream)
        .section(SECTION_KIND_INLINE_DATA, 5, &inline)
        .section(frame_wire::SECTION_KIND_TIMING, 3, &timing)
        .build();

    let frame = validate(&bytes).expect("valid");
    let base = bytes.as_ptr() as usize;
    for section in frame.sections() {
        let start = section.bytes.as_ptr() as usize;
        assert!(start >= base, "section starts before the packet");
        assert!(
            start + section.bytes.len() <= base + bytes.len(),
            "section ends after the packet",
        );
    }
}
