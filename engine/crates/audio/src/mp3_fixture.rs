//! A synthetic MPEG-1 Layer III bitstream, built byte by byte for tests.
//!
//! **Why synthesised rather than checked in.** The properties these tests need
//! are not properties of any particular recording: that a chunked decode equals
//! an unchunked one, that a frame whose main data lives in the previous frame's
//! bit reservoir still decodes after a chunk boundary, and that a steady chunk
//! costs no allocations. A fixture built here states its own structure, so a
//! test that depends on the reservoir can say so in the bytes rather than hope a
//! recording happens to contain one.
//!
//! Every frame decodes to silence, which is deliberate: the assertions are about
//! framing and decoder state, and silence makes "no samples at all" -- the
//! symptom of a lost reservoir -- unmistakable next to "1152 samples of zero".

/// MPEG-1, Layer III, 128 kb/s, 44.1 kHz, stereo, no CRC, no padding.
///
/// minimp3 derives the frame length from these four bytes as
/// `1152 * 128 * 125 / 44100 = 417`.
const HEADER: [u8; 4] = [0xFF, 0xFB, 0x90, 0x00];

/// Total bytes in one frame of the stream above.
pub(crate) const FRAME_BYTES: usize = 417;

/// Side info is 32 bytes for an MPEG-1 stereo frame: 9 bits of
/// `main_data_begin`, 11 bits of private/scfsi, then four 59-bit granule
/// descriptions. Left zero, every granule declares `part2_3_length = 0`, which
/// is a well-formed frame that carries no spectral data.
const SIDE_INFO_BYTES: usize = 32;

pub(crate) const SAMPLE_RATE: u32 = 44_100;
pub(crate) const CHANNELS: usize = 2;
pub(crate) const SAMPLES_PER_FRAME: usize = 1152 * CHANNELS;

/// How far back into the reservoir the frames after the first point.
///
/// Any non-zero value works. What matters is that a decoder which has just been
/// reset holds nothing, so `L3_restore_reservoir` refuses and the frame produces
/// no samples at all — which is exactly the failure a per-chunk decoder caused.
const MAIN_DATA_BEGIN: u16 = 8;

fn frame(main_data_begin: u16) -> [u8; FRAME_BYTES] {
    let mut bytes = [0u8; FRAME_BYTES];
    bytes[..HEADER.len()].copy_from_slice(&HEADER);

    // `main_data_begin` is the first 9 bits of the side info, most significant
    // bit first: eight in the first byte, the last in the top bit of the second.
    let side = HEADER.len();
    bytes[side] = (main_data_begin >> 1) as u8;
    bytes[side + 1] = ((main_data_begin & 1) << 7) as u8;
    debug_assert!(FRAME_BYTES > HEADER.len() + SIDE_INFO_BYTES);

    bytes
}

/// `count` frames. The first is self-contained; every later one declares that
/// its main data starts in its predecessor's reservoir.
pub(crate) fn stream(count: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(count * FRAME_BYTES);
    for index in 0..count {
        let main_data_begin = if index == 0 { 0 } else { MAIN_DATA_BEGIN };
        bytes.extend_from_slice(&frame(main_data_begin));
    }
    bytes
}
