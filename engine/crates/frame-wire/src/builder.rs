//! A packet builder, for tests and for the golden corpus generator.
//!
//! Not the production producer. The production producer is JavaScript running
//! inside WebContent, and this exists so the Rust side can generate the fixed
//! corpus both encoders are checked against. Two encoders that agree with each
//! other but not with a corpus is a bug that only shows up on a device.

use crate::{
    FLAG_KNOWN_MASK, HEADER_BYTES, OFF_CHECKSUM, OFF_FLAGS, OFF_FRAME_ID, OFF_HEADER_BYTES,
    OFF_MAGIC, OFF_RESERVED0, OFF_RESOURCE_EPOCH, OFF_RUNTIME_GENERATION, OFF_SECTION_COUNT,
    OFF_SEQUENCE, OFF_SESSION_NONCE, OFF_SURFACE_GENERATION, OFF_TOTAL_BYTES, OFF_WIRE_VERSION,
    SECTION_ALIGNMENT, SECTION_ENTRY_BYTES, WIRE_MAGIC, WIRE_VERSION, stamp_checksum,
};

/// One section to encode.
pub struct SectionInput<'a> {
    pub kind: u32,
    pub item_count: u32,
    pub bytes: &'a [u8],
}

/// Builds a well-formed packet. Every field is settable, including to values
/// the validator rejects: the negative corpus needs a builder that will produce
/// a bad packet on request, or the rejection paths are never exercised.
pub struct WireFrameBuilder<'a> {
    pub session_nonce: u64,
    pub sequence: u64,
    pub runtime_generation: u32,
    pub surface_generation: u32,
    pub resource_epoch: u32,
    pub frame_id: u32,
    pub flags: u32,
    sections: Vec<SectionInput<'a>>,
}

impl<'a> Default for WireFrameBuilder<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> WireFrameBuilder<'a> {
    pub fn new() -> Self {
        Self {
            session_nonce: 0,
            sequence: 1,
            runtime_generation: 1,
            surface_generation: 0,
            resource_epoch: 0,
            frame_id: 1,
            flags: FLAG_KNOWN_MASK & crate::FLAG_PRESENT,
            sections: Vec::new(),
        }
    }

    pub fn section(mut self, kind: u32, item_count: u32, bytes: &'a [u8]) -> Self {
        self.sections.push(SectionInput {
            kind,
            item_count,
            bytes,
        });
        self
    }

    /// Encode. Section payloads are laid out in the order they were added, each
    /// padded up to the 8-byte boundary the next one starts on.
    pub fn build(&self) -> Vec<u8> {
        let count = self.sections.len() as u32;
        let table_bytes = count * SECTION_ENTRY_BYTES;
        let mut offsets = Vec::with_capacity(self.sections.len());
        let mut cursor = HEADER_BYTES + table_bytes;
        cursor = align_up(cursor);
        for section in &self.sections {
            offsets.push(cursor);
            cursor += section.bytes.len() as u32;
            cursor = align_up(cursor);
        }
        let total = cursor;

        let mut out = vec![0u8; total as usize];
        put_u32(&mut out, OFF_MAGIC, WIRE_MAGIC);
        put_u32(&mut out, OFF_WIRE_VERSION, WIRE_VERSION);
        put_u32(&mut out, OFF_HEADER_BYTES, HEADER_BYTES);
        put_u32(&mut out, OFF_TOTAL_BYTES, total);
        put_u64(&mut out, OFF_SESSION_NONCE, self.session_nonce);
        put_u64(&mut out, OFF_SEQUENCE, self.sequence);
        put_u32(&mut out, OFF_RUNTIME_GENERATION, self.runtime_generation);
        put_u32(&mut out, OFF_SURFACE_GENERATION, self.surface_generation);
        put_u32(&mut out, OFF_RESOURCE_EPOCH, self.resource_epoch);
        put_u32(&mut out, OFF_FRAME_ID, self.frame_id);
        put_u32(&mut out, OFF_FLAGS, self.flags);
        put_u32(&mut out, OFF_SECTION_COUNT, count);
        put_u32(&mut out, OFF_RESERVED0, 0);

        for (index, section) in self.sections.iter().enumerate() {
            let entry = HEADER_BYTES as usize + index * SECTION_ENTRY_BYTES as usize;
            let offset = offsets[index];
            put_u32(&mut out, entry, section.kind);
            put_u32(&mut out, entry + 4, offset);
            put_u32(&mut out, entry + 8, section.bytes.len() as u32);
            put_u32(&mut out, entry + 12, section.item_count);
            let start = offset as usize;
            out[start..start + section.bytes.len()].copy_from_slice(section.bytes);
        }

        put_u32(&mut out, OFF_CHECKSUM, 0);
        stamp_checksum(&mut out);
        out
    }
}

fn align_up(value: u32) -> u32 {
    value.div_ceil(SECTION_ALIGNMENT) * SECTION_ALIGNMENT
}

fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
