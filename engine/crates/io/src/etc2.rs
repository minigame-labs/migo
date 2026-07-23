//! ETC2 RGB encoder for ingest-time texture transcoding.
//!
//! The runtime already consumes compressed textures end to end -- [`crate::ktx2`]
//! parses the container, `fast_image_decoder` recognises it, and the graphics
//! crate uploads the blocks with `glCompressedTexImage2D`. What was missing is
//! anything that *produces* them, so this is the ingest half: RGBA8 in,
//! `VK_FORMAT_ETC2_R8G8B8_UNORM_BLOCK` blocks out, to be wrapped by
//! [`crate::ktx2::write_ktx2`] and stored in the package.
//!
//! # Why only the individual mode
//!
//! ETC2 RGB has five block modes (individual, differential, T, H, planar). This
//! encoder emits only the *individual* mode. That is a deliberate, conformant
//! subset, not an approximation of the format: every ETC2 decoder must handle
//! it, because it is the mode ETC1 already had. Restricting the search keeps the
//! encoder small enough to be read and verified against the spec, which matters
//! more here than the last dB of quality -- this runs once per image at package
//! install, and its output is checked against an independent decoder in the
//! tests below rather than against itself.
//!
//! Reference: OpenGL ES 3.0 spec, "ETC Compressed Texture Image Formats".

/// Bytes in one encoded 4x4 ETC2 RGB block.
pub const ETC2_RGB_BLOCK_BYTES: usize = 8;

/// `VK_FORMAT_ETC2_R8G8B8_UNORM_BLOCK`, the format this encoder produces.
pub const VK_FORMAT_ETC2_R8G8B8_UNORM_BLOCK: u32 = 147;

/// Per-pixel modifier magnitudes, selected by a 3-bit codeword per sub-block.
///
/// A pixel picks one of four values: `±table[cw][0]` or `±table[cw][1]`.
const MODIFIER_TABLE: [[i32; 2]; 8] = [
    [2, 8],
    [5, 17],
    [9, 29],
    [13, 42],
    [18, 60],
    [24, 80],
    [33, 106],
    [47, 183],
];

/// Which sub-block each of the 16 pixel slots belongs to, per flip bit.
///
/// Indexed by *slot* order, not raster order: the format walks a block column by
/// column, which is what [`SLOT_TO_PIXEL`] undoes. flip=0 splits the block into
/// two 2x4 halves (left/right), flip=1 into two 4x2 halves (top/bottom).
const SUBBLOCK_TABLE: [[usize; 16]; 2] = [
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1],
    [0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1],
];

/// Slot order -> raster index within the 4x4 block (`y * 4 + x`).
const SLOT_TO_PIXEL: [usize; 16] = [0, 4, 8, 12, 1, 5, 9, 13, 2, 6, 10, 14, 3, 7, 11, 15];

/// Encode RGBA8 pixels as ETC2 RGB blocks. The alpha channel is ignored.
///
/// Blocks are emitted in raster order, 8 bytes each, which is the layout
/// `glCompressedTexImage2D` expects for level 0.
///
/// # Errors
///
/// Both dimensions must be non-zero multiples of 4. ETC2 has no partial blocks,
/// and silently padding would change the image the content addresses, so a
/// misaligned image is rejected for the caller to keep in its original form
/// rather than quietly altered.
pub fn encode_etc2_rgb(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, &'static str> {
    if width == 0 || height == 0 {
        return Err("etc2: zero dimensions");
    }
    if width % 4 != 0 || height % 4 != 0 {
        return Err("etc2: dimensions must be multiples of 4");
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|p| p.checked_mul(4))
        .ok_or("etc2: image dimensions overflow")?;
    if rgba.len() != expected {
        return Err("etc2: pixel buffer length does not match dimensions");
    }

    let blocks_x = (width / 4) as usize;
    let blocks_y = (height / 4) as usize;
    let mut out = Vec::with_capacity(blocks_x * blocks_y * ETC2_RGB_BLOCK_BYTES);

    let mut block = [[0u8; 3]; 16];
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            for y in 0..4 {
                for x in 0..4 {
                    let px = (by * 4 + y) * width as usize + (bx * 4 + x);
                    let base = px * 4;
                    block[y * 4 + x] = [rgba[base], rgba[base + 1], rgba[base + 2]];
                }
            }
            out.extend_from_slice(&encode_block(&block));
        }
    }

    Ok(out)
}

/// One candidate encoding of a sub-block: its quantized base colour, codeword,
/// per-slot selectors and the squared error it costs.
struct SubBlockFit {
    /// 4-bit-per-channel base colour, as stored in the block.
    quantized: [u8; 3],
    codeword: usize,
    /// `(magnitude_bit, sign_bit)` for each of the sub-block's 8 slots.
    selectors: [(u32, u32); 8],
    error: u64,
}

/// Encode a single 4x4 block, trying both flips and keeping the better one.
fn encode_block(pixels: &[[u8; 3]; 16]) -> [u8; ETC2_RGB_BLOCK_BYTES] {
    let mut best: Option<(u8, [SubBlockFit; 2], u64)> = None;

    for flip in 0..2u8 {
        let table = &SUBBLOCK_TABLE[flip as usize];
        let mut slots: [Vec<usize>; 2] = [Vec::with_capacity(8), Vec::with_capacity(8)];
        for (slot, &sub) in table.iter().enumerate() {
            slots[sub].push(slot);
        }

        let fit0 = fit_subblock(pixels, &slots[0]);
        let fit1 = fit_subblock(pixels, &slots[1]);
        let total = fit0.error + fit1.error;
        if best.as_ref().is_none_or(|(_, _, e)| total < *e) {
            best = Some((flip, [fit0, fit1], total));
        }
    }

    let (flip, fits, _) = best.expect("both flips are always evaluated");
    let table = &SUBBLOCK_TABLE[flip as usize];

    let mut bytes = [0u8; ETC2_RGB_BLOCK_BYTES];
    for channel in 0..3 {
        // High nibble is sub-block 0, low nibble sub-block 1; the decoder
        // expands each by replicating it into the low bits.
        bytes[channel] = (fits[0].quantized[channel] << 4) | fits[1].quantized[channel];
    }
    // bits 7..5 = sub-block 0 codeword, 4..2 = sub-block 1, bit 1 = diff (0 for
    // individual mode), bit 0 = flip.
    bytes[3] = ((fits[0].codeword as u8) << 5) | ((fits[1].codeword as u8) << 2) | flip;

    // Two 16-bit planes: `magnitude` picks which of the codeword's two
    // magnitudes a pixel uses, `sign` negates it. Bit i of each plane is slot i.
    let mut magnitude: u16 = 0;
    let mut sign: u16 = 0;
    let mut cursor = [0usize; 2];
    for (slot, &sub) in table.iter().enumerate() {
        let (mag_bit, sign_bit) = fits[sub].selectors[cursor[sub]];
        cursor[sub] += 1;
        magnitude |= (mag_bit as u16) << slot;
        sign |= (sign_bit as u16) << slot;
    }
    bytes[4..6].copy_from_slice(&sign.to_be_bytes());
    bytes[6..8].copy_from_slice(&magnitude.to_be_bytes());

    bytes
}

/// Choose the base colour, codeword and selectors for one sub-block.
///
/// The base colour is the sub-block average quantized to 4 bits per channel --
/// the only precision the individual mode has. The codeword is then chosen by
/// exhaustive search over all eight, which is cheap (8 codewords x 8 pixels x 4
/// candidates) and removes the main quality cliff a heuristic would introduce.
fn fit_subblock(pixels: &[[u8; 3]; 16], slots: &[usize]) -> SubBlockFit {
    let mut sum = [0u32; 3];
    for &slot in slots {
        let px = pixels[SLOT_TO_PIXEL[slot]];
        for channel in 0..3 {
            sum[channel] += px[channel] as u32;
        }
    }
    let count = slots.len() as u32;

    let mut quantized = [0u8; 3];
    let mut base = [0i32; 3];
    for channel in 0..3 {
        let average = (sum[channel] + count / 2) / count;
        // 4 bits stored, expanded by the decoder as (q << 4) | q, i.e. q * 17.
        let q = ((average + 8) / 17).min(15) as u8;
        quantized[channel] = q;
        base[channel] = ((q << 4) | q) as i32;
    }

    let mut best_codeword = 0usize;
    let mut best_error = u64::MAX;
    let mut best_selectors = [(0u32, 0u32); 8];

    for (codeword, magnitudes) in MODIFIER_TABLE.iter().enumerate() {
        let mut error = 0u64;
        let mut selectors = [(0u32, 0u32); 8];
        for (index, &slot) in slots.iter().enumerate() {
            let target = pixels[SLOT_TO_PIXEL[slot]];
            let mut pixel_best = u64::MAX;
            let mut pixel_selector = (0u32, 0u32);
            for mag_bit in 0..2u32 {
                for sign_bit in 0..2u32 {
                    let modifier = if sign_bit == 1 {
                        -magnitudes[mag_bit as usize]
                    } else {
                        magnitudes[mag_bit as usize]
                    };
                    let mut candidate = 0u64;
                    for channel in 0..3 {
                        let value = (base[channel] + modifier).clamp(0, 255);
                        let delta = value - target[channel] as i32;
                        candidate += (delta * delta) as u64;
                    }
                    if candidate < pixel_best {
                        pixel_best = candidate;
                        pixel_selector = (mag_bit, sign_bit);
                    }
                }
            }
            error += pixel_best;
            selectors[index] = pixel_selector;
        }
        if error < best_error {
            best_error = error;
            best_codeword = codeword;
            best_selectors = selectors;
        }
    }

    SubBlockFit {
        quantized,
        codeword: best_codeword,
        selectors: best_selectors,
        error: best_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ktx2::{parse_ktx2, write_ktx2, VkFormat};

    /// Decode with an independent implementation, never with our own encoder's
    /// inverse: a round trip through code that shares assumptions with the
    /// encoder would prove the two agree, not that the output is ETC2.
    fn decode_reference(blocks: &[u8], width: u32, height: u32) -> Vec<[u8; 3]> {
        let blocks_x = (width / 4) as usize;
        let blocks_y = (height / 4) as usize;
        let mut image = vec![[0u8; 3]; (width * height) as usize];
        let mut decoded = [0u32; 16];

        for by in 0..blocks_y {
            for bx in 0..blocks_x {
                let offset = (by * blocks_x + bx) * ETC2_RGB_BLOCK_BYTES;
                texture2ddecoder::decode_etc2_rgb_block(
                    &blocks[offset..offset + ETC2_RGB_BLOCK_BYTES],
                    &mut decoded,
                );
                for y in 0..4 {
                    for x in 0..4 {
                        // The decoder packs pixels as little-endian BGRA.
                        let p = decoded[y * 4 + x];
                        let rgb = [
                            ((p >> 16) & 0xff) as u8,
                            ((p >> 8) & 0xff) as u8,
                            (p & 0xff) as u8,
                        ];
                        image[(by * 4 + y) * width as usize + (bx * 4 + x)] = rgb;
                    }
                }
            }
        }
        image
    }

    fn peak_signal_to_noise(original: &[u8], decoded: &[[u8; 3]]) -> f64 {
        let mut squared_error = 0f64;
        for (index, pixel) in decoded.iter().enumerate() {
            for channel in 0..3 {
                let delta = original[index * 4 + channel] as f64 - pixel[channel] as f64;
                squared_error += delta * delta;
            }
        }
        let mean = squared_error / (decoded.len() * 3) as f64;
        if mean == 0.0 {
            return f64::INFINITY;
        }
        10.0 * (255.0f64 * 255.0 / mean).log10()
    }

    fn gradient(width: u32, height: u32) -> Vec<u8> {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                rgba.push((x * 255 / width.max(1)) as u8);
                rgba.push((y * 255 / height.max(1)) as u8);
                rgba.push(((x + y) * 255 / (width + height).max(1)) as u8);
                rgba.push(255);
            }
        }
        rgba
    }

    #[test]
    fn output_size_is_one_block_per_sixteen_pixels() {
        let rgba = gradient(16, 8);
        let blocks = encode_etc2_rgb(&rgba, 16, 8).expect("aligned image encodes");
        assert_eq!(blocks.len(), (16 / 4) * (8 / 4) * ETC2_RGB_BLOCK_BYTES);
    }

    #[test]
    fn misaligned_dimensions_are_rejected_rather_than_padded() {
        let rgba = gradient(6, 4);
        assert!(encode_etc2_rgb(&rgba, 6, 4).is_err());
        let rgba = gradient(4, 6);
        assert!(encode_etc2_rgb(&rgba, 4, 6).is_err());
    }

    #[test]
    fn pixel_buffer_length_must_match_dimensions() {
        let rgba = gradient(8, 8);
        assert!(encode_etc2_rgb(&rgba[..rgba.len() - 4], 8, 8).is_err());
    }

    #[test]
    fn a_flat_block_survives_the_round_trip_almost_exactly() {
        // A single colour needs no modifier at all, so the only loss is the
        // 4-bit quantization of the base colour: at most 8 per channel.
        let mut rgba = Vec::new();
        for _ in 0..16 {
            rgba.extend_from_slice(&[0x33, 0x88, 0xCC, 0xFF]);
        }
        let blocks = encode_etc2_rgb(&rgba, 4, 4).expect("encodes");
        let decoded = decode_reference(&blocks, 4, 4);
        for pixel in &decoded {
            assert!((pixel[0] as i32 - 0x33).abs() <= 8, "r drifted: {pixel:?}");
            assert!((pixel[1] as i32 - 0x88).abs() <= 8, "g drifted: {pixel:?}");
            assert!((pixel[2] as i32 - 0xCC).abs() <= 8, "b drifted: {pixel:?}");
        }
    }

    #[test]
    fn a_gradient_round_trips_through_an_independent_decoder() {
        let (width, height) = (64u32, 64u32);
        let rgba = gradient(width, height);
        let blocks = encode_etc2_rgb(&rgba, width, height).expect("encodes");
        let decoded = decode_reference(&blocks, width, height);

        let psnr = peak_signal_to_noise(&rgba, &decoded);
        // 30 dB is the usual floor for "no obvious artefacts" on photographic
        // content; a correct individual-mode encoder clears it comfortably on a
        // smooth gradient, while any bit-layout mistake collapses it far below.
        assert!(psnr > 30.0, "PSNR too low: {psnr:.2} dB");
    }

    #[test]
    fn sharp_edges_still_decode_to_the_right_side_of_the_edge() {
        // Two flat halves: the flip search should place the sub-block split on
        // the edge, so each half stays close to its own colour.
        let (width, height) = (8u32, 4u32);
        let mut rgba = Vec::new();
        for _ in 0..height {
            for x in 0..width {
                let c = if x < 4 { 0x20 } else { 0xE0 };
                rgba.extend_from_slice(&[c, c, c, 0xFF]);
            }
        }
        let blocks = encode_etc2_rgb(&rgba, width, height).expect("encodes");
        let decoded = decode_reference(&blocks, width, height);
        for y in 0..height as usize {
            for x in 0..width as usize {
                let pixel = decoded[y * width as usize + x];
                let expected = if x < 4 { 0x20 } else { 0xE0 };
                assert!(
                    (pixel[0] as i32 - expected).abs() <= 16,
                    "edge smeared at ({x},{y}): {pixel:?}"
                );
            }
        }
    }

    #[test]
    fn encoded_blocks_survive_the_ktx2_container() {
        let (width, height) = (16u32, 16u32);
        let rgba = gradient(width, height);
        let blocks = encode_etc2_rgb(&rgba, width, height).expect("encodes");

        let container = write_ktx2(VK_FORMAT_ETC2_R8G8B8_UNORM_BLOCK, width, height, &blocks);
        let parsed = parse_ktx2(&container).expect("the runtime parser accepts what we write");

        assert_eq!(parsed.header.format, VkFormat::Etc2R8G8B8UnormBlock);
        assert_eq!(parsed.header.width, width);
        assert_eq!(parsed.header.height, height);
        assert_eq!(parsed.data, &blocks[..]);
    }
}
