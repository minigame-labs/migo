//! The `WireFramePacket`: one frame of drawing work, crossing a process
//! boundary, produced by code we do not control.
//!
//! # Why this is not `FramePacket`
//!
//! [`shared::protocol::frame_packet::FramePacket`] is the in-process shape: a
//! Rust enum holding pooled `Vec`s of commands, handed from the JavaScript
//! thread to the render thread. Its comments call that hand-off an IPC, and in
//! process terms it is not one -- both ends are the same address space, so the
//! `Vec` pointers inside it are meaningful on both sides.
//!
//! On iOS the producer is a Worker inside WebKit's WebContent process. Nothing
//! in that packet survives the trip: not the pointers, not the enum layout, not
//! the padding. So this crate defines the shape that does -- explicit
//! little-endian, offset-driven, self-describing, and validatable without
//! knowing anything about the producer.
//!
//! # Threat model
//!
//! The producer is the game's own JavaScript. It is not hostile by assumption,
//! but it is arbitrary: a bug in a shipped game, or a compromised content
//! bundle, reaches this parser with whatever bytes it likes. So the rules are
//! the ones for parsing anything untrusted:
//!
//! - validate before allocating, and before creating any GPU object;
//! - never panic, never index out of bounds, never loop unboundedly, never
//!   allocate proportionally to an attacker-chosen count;
//! - unknown *required* structure is a rejection, never a silent skip.
//!
//! Validation here touches only the envelope: header, section table, bounds and
//! checksum. The command payload inside a section is validated separately by
//! the stream validator that already exists for WebGL, which is a pure function
//! over `&[u32]`. Keeping the two apart is deliberate: this layer must stay
//! correct without knowing which opcodes exist this month.
//!
//! # Alignment
//!
//! Nothing here requires the input to be aligned. The bytes arrive from Swift
//! inside `Data.withUnsafeBytes`, whose base pointer carries no alignment
//! guarantee, so every multi-byte read goes through `from_le_bytes` on a byte
//! subslice rather than a pointer cast. A cast would work on every machine we
//! test on and fault on one we do not.

#![forbid(unsafe_code)]

use core::fmt;

/// `MGPF`, matching the convention the WebGL command stream already uses for
/// its own `MGL1`: the magic is compared as the little-endian `u32` the
/// producer writes, not as a byte string.
pub const WIRE_MAGIC: u32 = 0x4D47_5046;

/// Bumped only for a change no v1 reader could survive. Additive change goes
/// through new section kinds, which have their own forward-compatibility rule.
pub const WIRE_VERSION: u32 = 1;

/// Fixed, and validated exactly rather than trusted from the packet: a header
/// that announces its own length lets a producer move the section table.
pub const HEADER_BYTES: u32 = 64;

/// One section-table entry.
pub const SECTION_ENTRY_BYTES: u32 = 16;

/// Every section starts on an 8-byte boundary so a reader may hand a payload
/// straight to code that wants `u64`-aligned access without copying.
pub const SECTION_ALIGNMENT: u32 = 8;

/// There are six kinds. A packet with more sections than that is malformed by
/// construction, and the cap is what stops a 4-billion-entry table from
/// becoming a loop the parser runs before it notices.
pub const MAX_SECTIONS: u32 = 8;

/// 64 MiB. Far above any real frame -- a heavy WebGL frame measures in tens of
/// kilobytes -- and small enough that a bogus `total_bytes` is rejected rather
/// than believed.
pub const MAX_TOTAL_BYTES: u32 = 64 * 1024 * 1024;

/// Section kinds.
///
/// A kind with [`SECTION_KIND_ADVISORY_BIT`] set may be skipped by a reader
/// that does not know it; any other unknown kind is a rejection. Silently
/// ignoring an unknown required section is how a reader ends up drawing a frame
/// that is missing the resource binding it depended on.
pub const SECTION_KIND_COMMAND_STREAM: u32 = 1;
pub const SECTION_KIND_INLINE_DATA: u32 = 2;
pub const SECTION_KIND_RESOURCE_REFERENCES: u32 = 3;
pub const SECTION_KIND_DAMAGE: u32 = 0x8000_0001;
pub const SECTION_KIND_TIMING: u32 = 0x8000_0002;

/// Kinds at or above this are advisory: a reader that does not recognise one
/// skips it. Below it, an unrecognised kind is fatal.
pub const SECTION_KIND_ADVISORY_BIT: u32 = 0x8000_0000;

/// Set when this packet ends a frame and the renderer should present.
pub const FLAG_PRESENT: u32 = 1 << 0;
/// Set when the producer knows more packets belong to this `frame_id`.
pub const FLAG_CONTINUED: u32 = 1 << 1;

pub(crate) const FLAG_KNOWN_MASK: u32 = FLAG_PRESENT | FLAG_CONTINUED;

// Header field offsets. Named rather than open-coded so the layout is one list
// that can be read against the wire format documentation.
pub(crate) const OFF_MAGIC: usize = 0;
pub(crate) const OFF_WIRE_VERSION: usize = 4;
pub(crate) const OFF_HEADER_BYTES: usize = 8;
pub(crate) const OFF_TOTAL_BYTES: usize = 12;
pub(crate) const OFF_SESSION_NONCE: usize = 16;
pub(crate) const OFF_SEQUENCE: usize = 24;
pub(crate) const OFF_RUNTIME_GENERATION: usize = 32;
pub(crate) const OFF_SURFACE_GENERATION: usize = 36;
pub(crate) const OFF_RESOURCE_EPOCH: usize = 40;
pub(crate) const OFF_FRAME_ID: usize = 44;
pub(crate) const OFF_FLAGS: usize = 48;
pub(crate) const OFF_SECTION_COUNT: usize = 52;
pub(crate) const OFF_CHECKSUM: usize = 56;
pub(crate) const OFF_RESERVED0: usize = 60;

/// Why a packet was rejected.
///
/// Stable numbers: they cross the C ABI to the host, which reports them in
/// telemetry, so a reader six months from now can tell "the producer sent a
/// short packet" apart from "the producer sent someone else's packet". Never
/// renumber; only append.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum WireError {
    ShorterThanHeader = 1,
    BadMagic = 2,
    UnsupportedVersion = 3,
    BadHeaderBytes = 4,
    BadTotalBytes = 5,
    LengthMismatch = 6,
    TooManySections = 7,
    SectionTableOutOfBounds = 8,
    SectionOutOfBounds = 9,
    SectionMisaligned = 10,
    SectionsOverlapOrUnordered = 11,
    DuplicateSection = 12,
    UnknownRequiredSection = 13,
    ChecksumMismatch = 14,
    UnknownFlags = 15,
    ReservedNotZero = 16,
    CommandStreamNotWordAligned = 17,
    ItemCountExceedsSection = 18,
    MissingCommandStream = 19,
}

impl WireError {
    #[inline]
    pub const fn code(self) -> u32 {
        self as u32
    }
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ShorterThanHeader => "packet is shorter than the fixed header",
            Self::BadMagic => "magic is not MGPF",
            Self::UnsupportedVersion => "wire version is not supported by this reader",
            Self::BadHeaderBytes => "header_bytes does not equal the fixed header size",
            Self::BadTotalBytes => "total_bytes is below the header or above the cap",
            Self::LengthMismatch => "total_bytes does not equal the delivered byte count",
            Self::TooManySections => "section_count exceeds the cap",
            Self::SectionTableOutOfBounds => "the section table does not fit in the packet",
            Self::SectionOutOfBounds => "a section extends past the end of the packet",
            Self::SectionMisaligned => "a section offset is not 8-byte aligned",
            Self::SectionsOverlapOrUnordered => "sections are not disjoint and ascending",
            Self::DuplicateSection => "the same section kind appears twice",
            Self::UnknownRequiredSection => "an unknown non-advisory section kind is present",
            Self::ChecksumMismatch => "the payload checksum does not match",
            Self::UnknownFlags => "an unknown flag bit is set",
            Self::ReservedNotZero => "a reserved field is not zero",
            Self::CommandStreamNotWordAligned => {
                "the command stream is not a whole number of words"
            }
            Self::ItemCountExceedsSection => "item_count is larger than the section can hold",
            Self::MissingCommandStream => "the packet carries no command stream",
        })
    }
}

/// One validated section: a kind and a borrowed slice of the original bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Section<'a> {
    pub kind: u32,
    pub item_count: u32,
    pub bytes: &'a [u8],
}

/// A packet whose envelope has been fully validated.
///
/// Holding one is the proof: every offset inside is in range, the checksum
/// matched, and no field needs re-checking downstream. Nothing constructs this
/// except [`validate`].
#[derive(Clone, Copy, Debug)]
pub struct WireFrame<'a> {
    bytes: &'a [u8],
    session_nonce: u64,
    sequence: u64,
    runtime_generation: u32,
    surface_generation: u32,
    resource_epoch: u32,
    frame_id: u32,
    flags: u32,
    section_count: u32,
}

impl<'a> WireFrame<'a> {
    #[inline]
    pub const fn session_nonce(&self) -> u64 {
        self.session_nonce
    }
    #[inline]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
    #[inline]
    pub const fn runtime_generation(&self) -> u32 {
        self.runtime_generation
    }
    #[inline]
    pub const fn surface_generation(&self) -> u32 {
        self.surface_generation
    }
    #[inline]
    pub const fn resource_epoch(&self) -> u32 {
        self.resource_epoch
    }
    #[inline]
    pub const fn frame_id(&self) -> u32 {
        self.frame_id
    }
    #[inline]
    pub const fn flags(&self) -> u32 {
        self.flags
    }
    #[inline]
    pub const fn presents(&self) -> bool {
        self.flags & FLAG_PRESENT != 0
    }
    #[inline]
    pub const fn section_count(&self) -> u32 {
        self.section_count
    }
    #[inline]
    pub const fn total_bytes(&self) -> usize {
        self.bytes.len()
    }

    /// Every section, in the ascending offset order validation established.
    pub fn sections(&self) -> impl Iterator<Item = Section<'a>> + '_ {
        let bytes = self.bytes;
        (0..self.section_count).map(move |index| {
            // Every read below was bounds-checked by `validate`; the slicing
            // here cannot be out of range for a `WireFrame` that exists.
            let entry = HEADER_BYTES as usize + (index * SECTION_ENTRY_BYTES) as usize;
            let kind = read_u32(bytes, entry);
            let offset = read_u32(bytes, entry + 4) as usize;
            let length = read_u32(bytes, entry + 8) as usize;
            let item_count = read_u32(bytes, entry + 12);
            Section {
                kind,
                item_count,
                bytes: &bytes[offset..offset + length],
            }
        })
    }

    /// The command stream, which every drawing packet must carry.
    pub fn command_stream(&self) -> Option<Section<'a>> {
        self.sections()
            .find(|section| section.kind == SECTION_KIND_COMMAND_STREAM)
    }
}

#[inline]
fn read_u32(bytes: &[u8], at: usize) -> u32 {
    // Byte-wise on purpose: see the alignment note in the module docs.
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

#[inline]
fn read_u64(bytes: &[u8], at: usize) -> u64 {
    let mut buffer = [0u8; 8];
    buffer.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(buffer)
}

/// Validate one packet's envelope. Allocates nothing, and cannot panic for any
/// input.
///
/// The order is chosen so cheap, high-signal checks reject first and the
/// checksum -- the only pass over the whole packet -- runs last. A malformed
/// header should never cost a full CRC.
pub fn validate(bytes: &[u8]) -> Result<WireFrame<'_>, WireError> {
    if bytes.len() < HEADER_BYTES as usize {
        return Err(WireError::ShorterThanHeader);
    }

    if read_u32(bytes, OFF_MAGIC) != WIRE_MAGIC {
        return Err(WireError::BadMagic);
    }
    if read_u32(bytes, OFF_WIRE_VERSION) != WIRE_VERSION {
        return Err(WireError::UnsupportedVersion);
    }
    if read_u32(bytes, OFF_HEADER_BYTES) != HEADER_BYTES {
        return Err(WireError::BadHeaderBytes);
    }
    if read_u32(bytes, OFF_RESERVED0) != 0 {
        return Err(WireError::ReservedNotZero);
    }

    let total_bytes = read_u32(bytes, OFF_TOTAL_BYTES);
    if !(HEADER_BYTES..=MAX_TOTAL_BYTES).contains(&total_bytes) {
        return Err(WireError::BadTotalBytes);
    }
    // The delivered length is the authority, and the announced one must agree
    // exactly. Accepting a packet longer than it claims would leave trailing
    // bytes outside the checksum -- a place to hide anything.
    if total_bytes as usize != bytes.len() {
        return Err(WireError::LengthMismatch);
    }

    let flags = read_u32(bytes, OFF_FLAGS);
    if flags & !FLAG_KNOWN_MASK != 0 {
        return Err(WireError::UnknownFlags);
    }

    let section_count = read_u32(bytes, OFF_SECTION_COUNT);
    if section_count > MAX_SECTIONS {
        return Err(WireError::TooManySections);
    }

    // Checked arithmetic throughout: `section_count * 16` overflows a u32 for
    // large counts, and an overflowed table end compares as "fits".
    let table_bytes = section_count
        .checked_mul(SECTION_ENTRY_BYTES)
        .ok_or(WireError::SectionTableOutOfBounds)?;
    let payload_start = HEADER_BYTES
        .checked_add(table_bytes)
        .ok_or(WireError::SectionTableOutOfBounds)?;
    if payload_start > total_bytes {
        return Err(WireError::SectionTableOutOfBounds);
    }

    // Fixed-size, because MAX_SECTIONS bounds it. A Vec here would be an
    // allocation sized by an attacker-supplied count, inside the parser whose
    // job is to not do that.
    let mut seen_kinds = [0u32; MAX_SECTIONS as usize];
    let mut previous_end = payload_start;
    let mut has_command_stream = false;

    for index in 0..section_count {
        let entry = HEADER_BYTES as usize + (index * SECTION_ENTRY_BYTES) as usize;
        let kind = read_u32(bytes, entry);
        let offset = read_u32(bytes, entry + 4);
        let length = read_u32(bytes, entry + 8);
        let item_count = read_u32(bytes, entry + 12);

        if kind < SECTION_KIND_ADVISORY_BIT
            && !matches!(
                kind,
                SECTION_KIND_COMMAND_STREAM
                    | SECTION_KIND_INLINE_DATA
                    | SECTION_KIND_RESOURCE_REFERENCES
            )
        {
            return Err(WireError::UnknownRequiredSection);
        }

        // `index` is the count of kinds already recorded: every earlier
        // iteration either returned or wrote exactly one entry.
        let recorded = index as usize;
        for &previous in &seen_kinds[..recorded] {
            if previous == kind {
                return Err(WireError::DuplicateSection);
            }
        }
        seen_kinds[recorded] = kind;

        if !offset.is_multiple_of(SECTION_ALIGNMENT) {
            return Err(WireError::SectionMisaligned);
        }
        // Ascending and disjoint. Overlapping sections would let one byte range
        // be read under two different kinds -- the shape of an aliasing bug the
        // consumer cannot defend against on its own.
        if offset < previous_end {
            return Err(WireError::SectionsOverlapOrUnordered);
        }
        let end = offset
            .checked_add(length)
            .ok_or(WireError::SectionOutOfBounds)?;
        if end > total_bytes {
            return Err(WireError::SectionOutOfBounds);
        }
        previous_end = end;

        if kind == SECTION_KIND_COMMAND_STREAM {
            has_command_stream = true;
            if !length.is_multiple_of(4) {
                return Err(WireError::CommandStreamNotWordAligned);
            }
            // Each record is at least one word, so a claimed record count above
            // the word count is a lie the consumer would otherwise carry into
            // its own loop bound.
            if item_count > length / 4 {
                return Err(WireError::ItemCountExceedsSection);
            }
        } else if item_count as u64 > length as u64 {
            return Err(WireError::ItemCountExceedsSection);
        }
    }

    if !has_command_stream {
        return Err(WireError::MissingCommandStream);
    }

    if checksum(bytes) != read_u32(bytes, OFF_CHECKSUM) {
        return Err(WireError::ChecksumMismatch);
    }

    Ok(WireFrame {
        bytes,
        session_nonce: read_u64(bytes, OFF_SESSION_NONCE),
        sequence: read_u64(bytes, OFF_SEQUENCE),
        runtime_generation: read_u32(bytes, OFF_RUNTIME_GENERATION),
        surface_generation: read_u32(bytes, OFF_SURFACE_GENERATION),
        resource_epoch: read_u32(bytes, OFF_RESOURCE_EPOCH),
        frame_id: read_u32(bytes, OFF_FRAME_ID),
        flags,
        section_count,
    })
}

/// CRC32 of the whole packet with the checksum field itself read as zero.
///
/// Covering the header, not just the payload, is the point: a flipped
/// `section_count` or `frame_id` is exactly the kind of corruption that still
/// parses. The producer computes this the same way, over the same bytes.
pub fn checksum(bytes: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&bytes[..OFF_CHECKSUM]);
    hasher.update(&[0u8; 4]);
    if bytes.len() > OFF_RESERVED0 {
        hasher.update(&bytes[OFF_RESERVED0..]);
    }
    hasher.finalize()
}

/// Write the checksum into a packet that is otherwise complete.
///
/// Lives here rather than in the producer so both ends compute it from one
/// implementation. Two encoders that agree with each other and not with a fixed
/// corpus is the failure this avoids.
pub fn stamp_checksum(bytes: &mut [u8]) {
    if bytes.len() < HEADER_BYTES as usize {
        return;
    }
    let value = checksum(bytes).to_le_bytes();
    bytes[OFF_CHECKSUM..OFF_CHECKSUM + 4].copy_from_slice(&value);
}

/// The command stream carried inside a `COMMAND_STREAM` section.
///
/// It lives here, and not in the JavaScript runtime where it was written,
/// because both readers need it and only one of them has a JavaScript engine.
/// In-process, `runtime-v8` validates the same words before decoding them; on
/// the cross-process path, the consumer validates them with no V8 linked at all
/// -- which is the product claim `MigoApplePerformancePlus` rests on and could
/// not make if this validator were reachable only through the engine crate.
///
/// The file moved unchanged. It had no imports at all, which is what made it
/// movable and is worth preserving: a pure function over `&[u32]` is the only
/// shape that can be shared by a trusted in-process caller and an untrusted
/// cross-process one without either inheriting the other's dependencies.
pub mod gl_stream;

pub mod ingress;
pub use ingress::{FrameIngress, IngressDecision, IngressOutcome};

#[cfg(any(test, feature = "test-support"))]
pub mod builder;
