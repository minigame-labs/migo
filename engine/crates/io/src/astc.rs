//! ASTC 4x4 LDR encoder for ingest-time texture transcoding.
//!
//! The counterpart of [`crate::etc2`], for the devices and the platform that
//! want ASTC: Apple GPUs take it as their native compressed format, and on
//! Android it is what a GPU advertising `GL_KHR_texture_compression_astc_ldr`
//! decodes. The runtime half has been in place for a while --
//! [`crate::ktx2`] parses the container, `fast_image_decoder` recognises it,
//! and the graphics crate uploads the blocks with `glCompressedTexImage2D`.
//!
//! # Why dual plane, and why that is not optional
//!
//! An ASTC 4x4 block is 16 bytes for 16 texels: exactly the size of an ETC2
//! RGBA block. So this format only earns its place if it is *better* at the
//! same size, and the naive configuration is not. In single-plane mode one
//! interpolation weight drives all four channels, so a sprite whose alpha edge
//! does not follow its colour edge is reconstructed worse than ETC2 RGBA, whose
//! alpha is an independent 8-bit EAC block. Dual-plane mode gives alpha its own
//! weight, which is what makes this an improvement rather than a regression.
//!
//! Dual plane doubles the weight bits, which is what forces the rest of the
//! configuration: 64 bits of weights leave 45 for eight endpoint values, so the
//! endpoints cannot be plain 8-bit and have to go through integer sequence
//! encoding. A trit plus four bits packs eight values into exactly 45.
//!
//! # The configuration, and why each part of it
//!
//! | | |
//! |---|---|
//! | footprint | 4x4 -- the only one whose weight grid can equal it without infill |
//! | partitions | 1 -- partitioning is for blocks with two distinct materials |
//! | colour endpoint mode | 12, LDR RGBA direct |
//! | weight grid | 4x4, one weight per texel, so no interpolation error |
//! | weight range | 0..3, two bits |
//! | planes | 2 -- alpha on plane 1 |
//! | endpoint range | 0..47, one trit and four bits |
//!
//! Everything else the format offers -- other footprints, partitioning, HDR,
//! void extents, the base+offset endpoint modes -- is deliberately absent. This
//! runs on the user's device at package ingest, so it has to be small and fast,
//! and a restricted encoder that is *checked* beats a general one that is
//! argued about. `scripts/test-astc-encoder.sh` decodes what this produces with
//! the platform's own ASTC decoder and compares it against the source pixels:
//! the check is against a third-party implementation, not against a second
//! reading of the specification by the same author.
//!
//! Reference: `KHR_texture_compression_astc_hdr`, sections C.2.8 through C.2.24.

/// Bytes in one encoded ASTC block, whatever the footprint.
pub const ASTC_BLOCK_BYTES: usize = 16;

/// `VK_FORMAT_ASTC_4x4_UNORM_BLOCK`, the format this encoder produces.
pub const VK_FORMAT_ASTC_4X4_UNORM_BLOCK: u32 = 157;

/// What went wrong, in the two ways it can.
#[derive(Debug, PartialEq, Eq)]
pub enum AstcError {
    /// `rgba` is not `width * height * 4` bytes.
    BadInputLength { expected: usize, actual: usize },
    /// A dimension is not a multiple of four.
    ///
    /// ASTC itself permits partial edge blocks, but the ingest path pairs this
    /// with a mip chain and a container whose levels are whole blocks, and the
    /// ETC2 encoder beside it draws the line in the same place. Two encoders
    /// that accept different images would make the sidecar's presence depend on
    /// which one ran.
    UnalignedDimensions { width: u32, height: u32 },
}

impl std::fmt::Display for AstcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadInputLength { expected, actual } => {
                write!(f, "expected {expected} bytes of RGBA, got {actual}")
            }
            Self::UnalignedDimensions { width, height } => {
                write!(f, "{width}x{height} is not a whole number of 4x4 blocks")
            }
        }
    }
}

// ── The specification's tables, derived rather than transcribed ─────────────

/// The five trits packed in an eight-bit `T` field, as the decoder reads them.
///
/// Transcribed from the decode procedure in C.2.12 rather than from an encoder
/// table, because inverting the decoder is what guarantees the encoder cannot
/// disagree with it. Some trit tuples have several `T` encodings; any of them
/// decodes correctly, so the first one found is kept.
const fn field(value: u8, hi: u32, lo: u32) -> u8 {
    (value >> lo) & ((1u8 << (hi - lo + 1)) - 1)
}

const fn decode_trits(t: u8) -> [u8; 5] {
    let (c, t4, t3);
    if field(t, 4, 2) == 0b111 {
        c = (field(t, 7, 5) << 2) | field(t, 1, 0);
        t4 = 2;
        t3 = 2;
    } else {
        c = field(t, 4, 0);
        if field(t, 6, 5) == 0b11 {
            t4 = 2;
            t3 = field(t, 7, 7);
        } else {
            t4 = field(t, 7, 7);
            t3 = field(t, 6, 5);
        }
    }
    let (t2, t1, t0);
    if field(c, 1, 0) == 0b11 {
        t2 = 2;
        t1 = field(c, 4, 4);
        t0 = (field(c, 3, 3) << 1) | (field(c, 2, 2) & !field(c, 3, 3) & 1);
    } else if field(c, 3, 2) == 0b11 {
        t2 = 2;
        t1 = 2;
        t0 = field(c, 1, 0);
    } else {
        t2 = field(c, 4, 4);
        t1 = field(c, 3, 2);
        t0 = (field(c, 1, 1) << 1) | (field(c, 0, 0) & !field(c, 1, 1) & 1);
    }
    [t0, t1, t2, t3, t4]
}

/// `[t0][t1][t2][t3][t4] -> T`, built by inverting [`decode_trits`].
static TRIT_ENCODE: std::sync::LazyLock<[u8; 243]> = std::sync::LazyLock::new(|| {
    let mut table = [u8::MAX; 243];
    for t in 0..=255u8 {
        let trits = decode_trits(t);
        let index = trits
            .iter()
            .rev()
            .fold(0usize, |acc, trit| acc * 3 + *trit as usize);
        if table[index] == u8::MAX {
            table[index] = t;
        }
    }
    debug_assert!(table.iter().all(|entry| *entry != u8::MAX));
    table
});

/// The value a stored endpoint level decodes to, for the range 0..47.
///
/// C.2.16 row `0..47  trit  4 bits  dcba`, followed by the shared
/// `T = D*C + B; T ^= A; T = (A & 0x80) | (T >> 2)`.
const fn endpoint_unquantised(level: u8) -> u8 {
    let trit = (level / 16) as u32;
    let m = (level % 16) as u32;
    let (a, b, c, d) = (m & 1, (m >> 1) & 1, (m >> 2) & 1, (m >> 3) & 1);
    let big_a = 0b1_1111_1111 * a;
    let big_b = (d << 8) | (c << 7) | (b << 6) | (d << 2) | (c << 1) | b;
    let mut value = trit * 22 + big_b;
    value ^= big_a;
    value = (big_a & 0x80) | (value >> 2);
    (value & 0xFF) as u8
}

/// Endpoint levels in stored order, which is not value order: the
/// unquantisation scrambles it, and C.2.13 says the encoder compensates with a
/// table.
static ENDPOINT_LEVELS: std::sync::LazyLock<[u8; 48]> = std::sync::LazyLock::new(|| {
    let mut levels = [0u8; 48];
    let mut level = 0;
    while level < 48 {
        levels[level] = endpoint_unquantised(level as u8);
        level += 1;
    }
    levels
});

/// The stored level whose decoded value is closest to `value`.
fn quantise_endpoint(value: u8) -> u8 {
    let mut best = 0u8;
    let mut best_error = i32::MAX;
    for (level, decoded) in ENDPOINT_LEVELS.iter().enumerate() {
        let error = (i32::from(*decoded) - i32::from(value)).pow(2);
        if error < best_error {
            best_error = error;
            best = level as u8;
        }
    }
    best
}

/// The weight a stored two-bit level decodes to, on the 0..64 scale the
/// interpolator divides by.
///
/// Bit replication into six bits, then C.2.17's final `if (T > 32) T += 1`,
/// which is what makes 64 rather than 63 the divisor.
const fn weight_unquantised(level: u8) -> u8 {
    let six = level * 0b010101;
    if six > 32 { six + 1 } else { six }
}

const WEIGHT_LEVELS: [u8; 4] = [
    weight_unquantised(0),
    weight_unquantised(1),
    weight_unquantised(2),
    weight_unquantised(3),
];

/// The block mode word for a 4x4 weight grid, weight range 0..3, dual plane.
///
/// Table C.2.8 row one: `D H B B A A R0 0 0 R2 R1`, width `B+4`, height `A+2`.
const fn block_mode() -> u32 {
    // The weight range index for 0..3, whose bits are scattered by the table.
    const R: u32 = 0b100;
    let (r0, r1, r2) = (R & 1, (R >> 1) & 1, (R >> 2) & 1);
    let (a, b) = (2u32, 0u32); // height 4, width 4
    // Bit 9 (H, high precision) and bits 3 and 2 (the row-one selector) are
    // zero, and are left out rather than written as `0 << n`: an or with zero
    // documents nothing the table above does not, and reads as a field being
    // set.
    (1 << 10)          // D: dual plane
        | (b << 7)
        | (a << 5)
        | (r0 << 4)
        | (r2 << 1)
        | r1
}

/// Pack eight endpoint levels as one trit plus four bits each: exactly 45 bits.
///
/// Figure C.5 with `n = 4`, and C.2.12's rule that a final partial block emits
/// only the bits its values need.
fn pack_endpoints(levels: &[u8; 8]) -> u128 {
    let mut out: u128 = 0;
    let mut width = 0u32;
    for chunk in levels.chunks(5) {
        let mut trits = [0u8; 5];
        let mut lows = [0u8; 5];
        for (index, level) in chunk.iter().enumerate() {
            trits[index] = level / 16;
            lows[index] = level % 16;
        }
        let index = trits
            .iter()
            .rev()
            .fold(0usize, |acc, trit| acc * 3 + *trit as usize);
        let t = u32::from(TRIT_ENCODE[index]);

        let mut block = 0u32;
        let mut at = 0u32;
        let mut put = |value: u32, bits: u32| {
            block |= (value & ((1 << bits) - 1)) << at;
            at += bits;
        };
        put(u32::from(lows[0]), 4);
        put(t, 2);
        put(u32::from(lows[1]), 4);
        put(t >> 2, 2);
        put(u32::from(lows[2]), 4);
        put(t >> 4, 1);
        put(u32::from(lows[3]), 4);
        put(t >> 5, 2);
        put(u32::from(lows[4]), 4);
        put(t >> 7, 1);

        let used = chunk.len() as u32;
        let keep = (used * 8).div_ceil(5) + used * 4;
        out |= u128::from(block & ((1u32 << keep) - 1)) << width;
        width += keep;
    }
    debug_assert_eq!(width, 45);
    out
}

/// One 4x4 block: two RGBA endpoints, a colour weight per texel, an alpha
/// weight per texel.
fn assemble_block(
    endpoints: [[u8; 4]; 2],
    colour_weights: [u8; 16],
    alpha_weights: [u8; 16],
) -> [u8; ASTC_BLOCK_BYTES] {
    let levels: [u8; 8] = [
        quantise_endpoint(endpoints[0][0]),
        quantise_endpoint(endpoints[1][0]),
        quantise_endpoint(endpoints[0][1]),
        quantise_endpoint(endpoints[1][1]),
        quantise_endpoint(endpoints[0][2]),
        quantise_endpoint(endpoints[1][2]),
        quantise_endpoint(endpoints[0][3]),
        quantise_endpoint(endpoints[1][3]),
    ];

    let mut block: u128 = u128::from(block_mode());
    // Partition count - 1 is zero, so bits 11 and 12 stay clear.
    block |= 12 << 13; // CEM 12: LDR RGBA, direct
    block |= pack_endpoints(&levels) << 17;
    // The colour component selector sits directly below the weight bits.
    // 3 selects alpha as the component carried on plane 1.
    block |= 3 << 62;

    // Weights grow downward from the most significant bit: bit n of the stream
    // is bit 127-n of the block. Both planes are emitted per location, plane 0
    // first.
    let mut stream: u64 = 0;
    let mut at = 0u32;
    for texel in 0..16 {
        stream |= u64::from(colour_weights[texel] & 3) << at;
        at += 2;
        stream |= u64::from(alpha_weights[texel] & 3) << at;
        at += 2;
    }
    debug_assert_eq!(at, 64);
    for bit in 0..64u32 {
        if (stream >> bit) & 1 == 1 {
            block |= 1u128 << (127 - bit);
        }
    }
    block.to_le_bytes()
}

/// The endpoints and weights for one 4x4 tile of RGBA pixels.
///
/// Colour and alpha are solved separately because the block interpolates them
/// separately: the colour endpoints are the extremes along the tile's dominant
/// RGB axis, and alpha is a one-dimensional problem of its own.
fn solve_tile(tile: &[[u8; 4]; 16]) -> ([[u8; 4]; 2], [u8; 16], [u8; 16]) {
    // The dominant axis, as the bounding box's longest side. A covariance
    // eigenvector is better on tiles whose colours lie along a diagonal, and
    // measurably so only on gradients; the box is what a 4x4 tile of real
    // content mostly is, and it costs no iteration.
    let mut low = [255u8; 3];
    let mut high = [0u8; 3];
    for pixel in tile {
        for channel in 0..3 {
            low[channel] = low[channel].min(pixel[channel]);
            high[channel] = high[channel].max(pixel[channel]);
        }
    }

    // The projection axis, and a scale that turns a projection into 0..1.
    let axis = [
        f32::from(high[0]) - f32::from(low[0]),
        f32::from(high[1]) - f32::from(low[1]),
        f32::from(high[2]) - f32::from(low[2]),
    ];
    let length = axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2];

    let mut colour_weights = [0u8; 16];
    if length > 0.0 {
        for (index, pixel) in tile.iter().enumerate() {
            let projection = (0..3)
                .map(|channel| {
                    (f32::from(pixel[channel]) - f32::from(low[channel])) * axis[channel]
                })
                .sum::<f32>()
                / length;
            colour_weights[index] = quantise_unit(projection);
        }
    }

    let alpha_low = tile.iter().map(|pixel| pixel[3]).min().unwrap_or(0);
    let alpha_high = tile.iter().map(|pixel| pixel[3]).max().unwrap_or(0);
    let mut alpha_weights = [0u8; 16];
    if alpha_high > alpha_low {
        let span = f32::from(alpha_high) - f32::from(alpha_low);
        for (index, pixel) in tile.iter().enumerate() {
            alpha_weights[index] =
                quantise_unit((f32::from(pixel[3]) - f32::from(alpha_low)) / span);
        }
    }

    let mut endpoints = [
        [low[0], low[1], low[2], alpha_low],
        [high[0], high[1], high[2], alpha_high],
    ];

    // Mode 12 applies blue contraction when the first endpoint's colour sum
    // exceeds the second's, which would decode to something this encoder did
    // not intend. Ordering the endpoints so it never triggers is exact: the
    // weight range is symmetric, so inverting the weights restores the
    // reconstruction the swap would otherwise change.
    // Compared on the *unquantised* values, which is what the decoder compares.
    // Comparing stored levels reads as equivalent and is not: C.2.13 says the
    // unquantisation "scrambles the order of the decoded values relative to the
    // encoded values", so a level ordering can disagree with a value ordering --
    // and when it does, this swaps, the decoder contracts, and the block decodes
    // to a colour neither side intended.
    let sum = |endpoint: &[u8; 4]| {
        u32::from(ENDPOINT_LEVELS[quantise_endpoint(endpoint[0]) as usize])
            + u32::from(ENDPOINT_LEVELS[quantise_endpoint(endpoint[1]) as usize])
            + u32::from(ENDPOINT_LEVELS[quantise_endpoint(endpoint[2]) as usize])
    };
    if sum(&endpoints[1]) < sum(&endpoints[0]) {
        endpoints.swap(0, 1);
        for weight in colour_weights.iter_mut().chain(alpha_weights.iter_mut()) {
            *weight = 3 - *weight;
        }
    }

    (endpoints, colour_weights, alpha_weights)
}

/// A 0..1 position on the endpoint segment, as the nearest stored weight.
fn quantise_unit(position: f32) -> u8 {
    let target = position.clamp(0.0, 1.0) * 64.0;
    let mut best = 0u8;
    let mut best_error = f32::MAX;
    for (level, decoded) in WEIGHT_LEVELS.iter().enumerate() {
        let error = (f32::from(*decoded) - target).abs();
        if error < best_error {
            best_error = error;
            best = level as u8;
        }
    }
    best
}

/// Encode an RGBA8 image as ASTC 4x4 LDR blocks, in raster block order.
pub fn encode_astc_4x4(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, AstcError> {
    let expected = (width as usize) * (height as usize) * 4;
    if rgba.len() != expected {
        return Err(AstcError::BadInputLength {
            expected,
            actual: rgba.len(),
        });
    }
    if width == 0 || height == 0 || !width.is_multiple_of(4) || !height.is_multiple_of(4) {
        return Err(AstcError::UnalignedDimensions { width, height });
    }

    let blocks_x = (width / 4) as usize;
    let blocks_y = (height / 4) as usize;
    let mut out = Vec::with_capacity(blocks_x * blocks_y * ASTC_BLOCK_BYTES);
    let mut tile = [[0u8; 4]; 16];

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            for row in 0..4 {
                let y = block_y * 4 + row;
                let base = (y * width as usize + block_x * 4) * 4;
                for column in 0..4 {
                    let at = base + column * 4;
                    tile[row * 4 + column] = [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]];
                }
            }
            let (endpoints, colour_weights, alpha_weights) = solve_tile(&tile);
            out.extend_from_slice(&assemble_block(endpoints, colour_weights, alpha_weights));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every trit tuple has an encoding, and it decodes back to itself.
    ///
    /// The table is built by inverting the specification's decoder, so this is
    /// what says the inversion is total: a tuple with no `T` would be a value
    /// the encoder cannot express, and it would show up as a wrong colour
    /// rather than as a failure.
    #[test]
    fn every_trit_tuple_round_trips() {
        for index in 0..243usize {
            let mut wanted = [0u8; 5];
            let mut rest = index;
            for trit in wanted.iter_mut() {
                *trit = (rest % 3) as u8;
                rest /= 3;
            }
            let encoded = TRIT_ENCODE[index];
            assert_eq!(
                decode_trits(encoded),
                wanted,
                "T={encoded} does not decode to {wanted:?}"
            );
        }
    }

    /// The block mode word says what this encoder means it to say.
    ///
    /// Decoded here against Table C.2.8 row one rather than compared to a
    /// number, because a number is exactly what a typo also is. A wrong grid
    /// size is not a build error, a link error, or a GL error: the decoder
    /// reads the block with the wrong shape and the texture is wrong.
    #[test]
    fn the_block_mode_describes_a_dual_plane_four_by_four_grid() {
        let mode = block_mode();
        assert!(mode < (1 << 11), "the block mode field is eleven bits");

        let bit = |at: u32| (mode >> at) & 1;
        assert_eq!(bit(10), 1, "D: dual plane");
        assert_eq!(bit(9), 0, "H: low precision range");
        assert_eq!((bit(3), bit(2)), (0, 0), "row one of the layout table");

        let b = (mode >> 7) & 0b11;
        let a = (mode >> 5) & 0b11;
        assert_eq!(b + 4, 4, "weight grid width");
        assert_eq!(a + 2, 4, "weight grid height");

        // R is scattered: R0 at bit 4, R2 at bit 1, R1 at bit 0.
        let r = (bit(1) << 2) | (bit(0) << 1) | bit(4);
        assert_eq!(r, 0b100, "the weight range index for 0..3");
    }

    /// The weight levels are the four the interpolator will see.
    #[test]
    fn the_weight_levels_span_the_full_zero_to_sixty_four_scale() {
        assert_eq!(WEIGHT_LEVELS, [0, 21, 43, 64]);
    }

    /// The endpoint range is fine enough that quantisation alone cannot account
    /// for a visible error.
    #[test]
    fn every_byte_has_a_close_endpoint_level() {
        let worst = (0..=255u8)
            .map(|value| {
                let level = quantise_endpoint(value);
                (i32::from(ENDPOINT_LEVELS[level as usize]) - i32::from(value)).abs()
            })
            .max()
            .expect("a non-empty range");
        assert!(
            worst <= 3,
            "the worst endpoint quantisation error is {worst}, which would put \
             the encoder's floor above what the round-trip gate allows"
        );
        // The ends must be exact: a black that decodes to 3 is a visible lift on
        // a dark scene, and an alpha of 252 composites wrong.
        assert_eq!(ENDPOINT_LEVELS[quantise_endpoint(0) as usize], 0);
        assert_eq!(ENDPOINT_LEVELS[quantise_endpoint(255) as usize], 255);
    }

    /// Eight endpoint values occupy exactly the space the configuration leaves.
    ///
    /// 128 bits, less eleven of block mode, two of partition count, four of
    /// colour endpoint mode, sixty-four of weights and two of plane selector,
    /// is forty-five. One bit more and the decoder would pick a different
    /// quantisation level than the encoder wrote.
    #[test]
    fn the_endpoints_fill_the_space_between_the_header_and_the_weights() {
        let payload = pack_endpoints(&[47, 0, 47, 0, 47, 0, 47, 0]);
        assert!(payload < (1u128 << 45), "the payload must fit in 45 bits");
        assert_eq!(128 - 11 - 2 - 4 - 64 - 2, 45);
    }

    #[test]
    fn a_short_buffer_is_refused_rather_than_read_past() {
        let error = encode_astc_4x4(&[0; 15], 4, 4).unwrap_err();
        assert_eq!(
            error,
            AstcError::BadInputLength {
                expected: 64,
                actual: 15
            }
        );
    }

    #[test]
    fn an_unaligned_image_is_refused() {
        let rgba = vec![0u8; 14 * 16 * 4];
        assert_eq!(
            encode_astc_4x4(&rgba, 14, 16).unwrap_err(),
            AstcError::UnalignedDimensions {
                width: 14,
                height: 16
            }
        );
    }

    #[test]
    fn the_output_is_one_block_per_tile() {
        let rgba = vec![0x80u8; 16 * 12 * 4];
        let blocks = encode_astc_4x4(&rgba, 16, 12).expect("aligned");
        assert_eq!(blocks.len(), (16 / 4) * (12 / 4) * ASTC_BLOCK_BYTES);
    }
}
