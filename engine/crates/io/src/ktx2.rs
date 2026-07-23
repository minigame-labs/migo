//! Zero-copy KTX2 container parser for compressed GPU textures.
//!
//! Parses the KTX2 fixed-size header and extracts the level 0 (base mip)
//! compressed texture data pointer without any allocation or copy.
//!
//! Only ETC2 and ASTC formats are recognized; everything else maps to
//! `VkFormat::Unknown(code)`.
//!
//! Reference: <https://registry.khronos.org/KTX/specs/2.0/ktxspec.v2.html>

/// KTX2 file identifier (12 bytes).
const KTX2_MAGIC: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];

/// Subset of Vulkan format codes relevant to mobile compressed textures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VkFormat {
    /// `VK_FORMAT_ETC2_R8G8B8_UNORM_BLOCK` (147)
    Etc2R8G8B8UnormBlock,
    /// `VK_FORMAT_ETC2_R8G8B8A8_UNORM_BLOCK` (151)
    Etc2R8G8B8A8UnormBlock,
    /// `VK_FORMAT_ASTC_4x4_UNORM_BLOCK` (157)
    Astc4x4UnormBlock,
    /// `VK_FORMAT_ASTC_6x6_UNORM_BLOCK` (163)
    Astc6x6UnormBlock,
    /// `VK_FORMAT_ASTC_8x8_UNORM_BLOCK` (169)
    Astc8x8UnormBlock,
    /// Unrecognized Vulkan format code.
    Unknown(u32),
}

impl VkFormat {
    fn from_u32(code: u32) -> Self {
        match code {
            147 => Self::Etc2R8G8B8UnormBlock,
            151 => Self::Etc2R8G8B8A8UnormBlock,
            157 => Self::Astc4x4UnormBlock,
            163 => Self::Astc6x6UnormBlock,
            169 => Self::Astc8x8UnormBlock,
            other => Self::Unknown(other),
        }
    }
}

/// Parsed KTX2 header fields needed for GPU upload.
#[derive(Debug, Clone)]
pub struct Ktx2Header {
    pub format: VkFormat,
    pub width: u32,
    pub height: u32,
    pub mip_levels: u32,
}

/// A parsed KTX2 file with a borrowed slice to the level 0 compressed data.
#[derive(Debug)]
pub struct Ktx2File<'a> {
    pub header: Ktx2Header,
    /// Compressed texture data for mip level 0.
    pub data: &'a [u8],
}

/// KTX2 header layout (offsets from the spec, all little-endian):
///
/// | Offset | Size | Field                |
/// |--------|------|----------------------|
/// | 0      | 12   | identifier (magic)   |
/// | 12     | 4    | vkFormat             |
/// | 16     | 4    | typeSize             |
/// | 20     | 4    | pixelWidth           |
/// | 24     | 4    | pixelHeight          |
/// | 28     | 4    | pixelDepth           |
/// | 32     | 4    | layerCount           |
/// | 36     | 4    | faceCount            |
/// | 40     | 4    | levelCount           |
/// | 44     | 4    | supercompressionScheme |
///
/// After the 48-byte header comes the index section:
///
/// | 48     | 4    | dfdByteOffset        |
/// | 52     | 4    | dfdByteLength        |
/// | 56     | 4    | kvdByteOffset        |
/// | 60     | 4    | kvdByteLength        |
/// | 64     | 8    | sgdByteOffset        |
/// | 72     | 8    | sgdByteLength        |
///
/// Then the level index (one entry per mip level):
///
/// | 80 + i*24 | 8  | byteOffset  |
/// | 88 + i*24 | 8  | byteLength  |
/// | 96 + i*24 | 8  | uncompressedByteLength |
const HEADER_SIZE: usize = 80;
const LEVEL_INDEX_ENTRY_SIZE: usize = 24;

/// Read a little-endian u32 from `data` at `offset`.
fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

/// Read a little-endian u64 from `data` at `offset`.
fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ])
}

/// Parse a KTX2 container, returning the header and a borrowed slice to the
/// level 0 compressed data.
///
/// Only the base mip level (level 0) is extracted. Supercompression is not
/// supported -- the data must be stored uncompressed in the container.
///
/// # Errors
///
/// Returns an error string if:
/// - The magic bytes do not match.
/// - The file is truncated.
/// - Supercompression is used (we only handle `None` = 0).
/// - The level 0 data range falls outside the input buffer.
pub fn parse_ktx2(data: &[u8]) -> Result<Ktx2File<'_>, &'static str> {
    // Check magic bytes.
    if data.len() < HEADER_SIZE {
        return Err("ktx2: file too short for header");
    }
    if data[..12] != KTX2_MAGIC {
        return Err("ktx2: invalid magic bytes");
    }

    let vk_format = read_u32(data, 12);
    let width = read_u32(data, 20);
    let height = read_u32(data, 24);
    let level_count = read_u32(data, 40);
    let supercompression_scheme = read_u32(data, 44);

    if width == 0 || height == 0 {
        return Err("ktx2: zero dimensions");
    }

    // We only support uncompressed-in-container (scheme 0).
    if supercompression_scheme != 0 {
        return Err("ktx2: supercompression not supported");
    }

    // Level count 0 means "auto full mip chain" per spec, but we only need
    // level 0 so treat it as at least 1.
    let actual_levels = if level_count == 0 { 1 } else { level_count };

    // Verify that the level index for level 0 fits in the buffer.
    let level_index_start = HEADER_SIZE;
    let level_index_end = level_index_start + (actual_levels as usize) * LEVEL_INDEX_ENTRY_SIZE;
    if data.len() < level_index_end {
        return Err("ktx2: file too short for level index");
    }

    // Level 0 entry is the first in the level index.
    let level0_offset = read_u64(data, level_index_start) as usize;
    let level0_length = read_u64(data, level_index_start + 8) as usize;

    if level0_length == 0 {
        return Err("ktx2: level 0 has zero length");
    }

    let level0_end = level0_offset
        .checked_add(level0_length)
        .ok_or("ktx2: level 0 range overflow")?;

    if level0_end > data.len() {
        return Err("ktx2: level 0 data out of bounds");
    }

    Ok(Ktx2File {
        header: Ktx2Header {
            format: VkFormat::from_u32(vk_format),
            width,
            height,
            mip_levels: actual_levels,
        },
        data: &data[level0_offset..level0_end],
    })
}

/// Check if the given data starts with the KTX2 magic bytes.
pub fn is_ktx2(data: &[u8]) -> bool {
    data.len() >= 12 && data[..12] == KTX2_MAGIC
}

/// Write a single-level, non-supercompressed KTX2 container around already
/// compressed block data.
///
/// This is the ingest-side counterpart of [`parse_ktx2`]: transcoding a source
/// image at package-install time produces block data that still needs a
/// container the runtime can recognise. Keeping the writer beside the parser is
/// deliberate -- they share the offset constants and the magic, so the layout
/// cannot drift apart, and `parse_ktx2` is the round-trip oracle for the writer.
///
/// Emits exactly what the parser needs and nothing else: no DFD, no key/value
/// data, no supercompression global data. Those sections are optional in the
/// spec, and every consumer in this engine reads only the header, the level
/// index and the level 0 bytes.
pub fn write_ktx2(vk_format: u32, width: u32, height: u32, level0: &[u8]) -> Vec<u8> {
    let level0_offset = (HEADER_SIZE + LEVEL_INDEX_ENTRY_SIZE) as u64;
    let level0_length = level0.len() as u64;

    let mut buf = vec![0u8; HEADER_SIZE + LEVEL_INDEX_ENTRY_SIZE + level0.len()];

    buf[..12].copy_from_slice(&KTX2_MAGIC);
    buf[12..16].copy_from_slice(&vk_format.to_le_bytes());
    // typeSize is 1 for every block-compressed format.
    buf[16..20].copy_from_slice(&1u32.to_le_bytes());
    buf[20..24].copy_from_slice(&width.to_le_bytes());
    buf[24..28].copy_from_slice(&height.to_le_bytes());
    // pixelDepth 0 = 2D, layerCount 0 = non-array, faceCount 1 = not a cubemap.
    buf[28..32].copy_from_slice(&0u32.to_le_bytes());
    buf[32..36].copy_from_slice(&0u32.to_le_bytes());
    buf[36..40].copy_from_slice(&1u32.to_le_bytes());
    buf[40..44].copy_from_slice(&1u32.to_le_bytes());
    // supercompressionScheme 0 = none; the parser rejects anything else.
    buf[44..48].copy_from_slice(&0u32.to_le_bytes());

    // The dfd/kvd/sgd index entries stay zero: those sections are absent.

    let li = HEADER_SIZE;
    buf[li..li + 8].copy_from_slice(&level0_offset.to_le_bytes());
    buf[li + 8..li + 16].copy_from_slice(&level0_length.to_le_bytes());
    // uncompressedByteLength equals byteLength when nothing is supercompressed.
    buf[li + 16..li + 24].copy_from_slice(&level0_length.to_le_bytes());

    buf[level0_offset as usize..].copy_from_slice(level0);

    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid KTX2 buffer for testing.
    ///
    /// Delegates to the production writer so these tests exercise the real
    /// layout rather than a second copy of it that could drift from it.
    fn make_test_ktx2(vk_format: u32, width: u32, height: u32, payload: &[u8]) -> Vec<u8> {
        write_ktx2(vk_format, width, height, payload)
    }

    #[test]
    fn parse_valid_etc2_rgb() {
        let payload = vec![0xAA; 64]; // fake compressed data
        let buf = make_test_ktx2(147, 256, 256, &payload);

        let ktx2 = parse_ktx2(&buf).expect("should parse");
        assert_eq!(ktx2.header.format, VkFormat::Etc2R8G8B8UnormBlock);
        assert_eq!(ktx2.header.width, 256);
        assert_eq!(ktx2.header.height, 256);
        assert_eq!(ktx2.header.mip_levels, 1);
        assert_eq!(ktx2.data, &payload[..]);
    }

    #[test]
    fn parse_valid_astc_4x4() {
        let payload = vec![0xBB; 128];
        let buf = make_test_ktx2(157, 512, 512, &payload);

        let ktx2 = parse_ktx2(&buf).expect("should parse");
        assert_eq!(ktx2.header.format, VkFormat::Astc4x4UnormBlock);
        assert_eq!(ktx2.header.width, 512);
        assert_eq!(ktx2.header.height, 512);
        assert_eq!(ktx2.data.len(), 128);
    }

    #[test]
    fn reject_bad_magic() {
        let buf = vec![0u8; 128];
        assert_eq!(parse_ktx2(&buf).unwrap_err(), "ktx2: invalid magic bytes");
    }

    #[test]
    fn reject_truncated() {
        assert_eq!(
            parse_ktx2(&[0xAB, 0x4B]).unwrap_err(),
            "ktx2: file too short for header"
        );
    }

    #[test]
    fn reject_supercompression() {
        let mut buf = make_test_ktx2(147, 4, 4, &[0; 8]);
        // Set supercompressionScheme to 1 (BasisLZ)
        buf[44..48].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            parse_ktx2(&buf).unwrap_err(),
            "ktx2: supercompression not supported"
        );
    }

    #[test]
    fn reject_zero_dimensions() {
        let buf = make_test_ktx2(147, 0, 256, &[0; 8]);
        assert_eq!(parse_ktx2(&buf).unwrap_err(), "ktx2: zero dimensions");
    }

    #[test]
    fn is_ktx2_detection() {
        let buf = make_test_ktx2(147, 4, 4, &[0; 8]);
        assert!(is_ktx2(&buf));
        assert!(!is_ktx2(&[0u8; 12]));
        assert!(!is_ktx2(&[0u8; 4]));
    }

    #[test]
    fn unknown_format_passthrough() {
        let payload = vec![0xCC; 32];
        let buf = make_test_ktx2(999, 64, 64, &payload);

        let ktx2 = parse_ktx2(&buf).expect("should parse");
        assert_eq!(ktx2.header.format, VkFormat::Unknown(999));
    }

    #[test]
    fn level_count_zero_treated_as_one() {
        let payload = vec![0xDD; 16];
        let mut buf = make_test_ktx2(147, 4, 4, &payload);
        // Set levelCount to 0
        buf[40..44].copy_from_slice(&0u32.to_le_bytes());

        let ktx2 = parse_ktx2(&buf).expect("should parse");
        assert_eq!(ktx2.header.mip_levels, 1);
        assert_eq!(ktx2.data, &payload[..]);
    }
}
