//! ASTC LDR encoder for ingest-time texture transcoding: 4x4, 6x6 and 8x8.
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
//! | footprint | 4x4, 6x6 or 8x8, chosen per image. 4x4 is the only one whose weight grid equals it, so it is the only one that needs no infill |
//! | partitions | 1 -- partitioning is for blocks with two distinct materials |
//! | colour endpoint mode | 12, LDR RGBA direct |
//! | weight grid | always 4x4 -- one weight per texel at the 4x4 footprint, bilinearly expanded by the decoder at 6x6 and 8x8 |
//! | weight range | 0..3, two bits |
//! | planes | 2 -- alpha on plane 1 |
//! | endpoint range | 0..47, one trit and four bits |
//!
//! Everything else the format offers -- partitioning, HDR, void extents, the
//! base+offset endpoint modes -- is deliberately absent. Partitioning is the
//! one with quality left in it: a single partition is a single colour line, so
//! a block holding two materials (a sprite edge, a texture seam) cannot be
//! expressed, which is what the low PSNR on `sprite-edge-8` is. This
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

/// `VK_FORMAT_ASTC_4x4_UNORM_BLOCK`.
pub const VK_FORMAT_ASTC_4X4_UNORM_BLOCK: u32 = 157;
/// `VK_FORMAT_ASTC_6x6_UNORM_BLOCK`.
pub const VK_FORMAT_ASTC_6X6_UNORM_BLOCK: u32 = 163;
/// `VK_FORMAT_ASTC_8x8_UNORM_BLOCK`.
pub const VK_FORMAT_ASTC_8X8_UNORM_BLOCK: u32 = 169;

/// The block footprints this encoder produces.
///
/// All three share one block layout, because the block mode field describes the
/// *weight grid* and not the footprint -- the footprint comes from the format
/// the texture is uploaded as. So the larger two cost only the weight infill
/// below, and every other part of the encoding is the same code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Footprint {
    /// 16 texels per block: one byte per pixel, one weight per texel.
    X4,
    /// 36 texels per block: 0.44 bytes per pixel, weights interpolated.
    X6,
    /// 64 texels per block: 0.25 bytes per pixel, weights interpolated.
    X8,
}

impl Footprint {
    /// Texels along each axis. Blocks are square in every footprint here.
    pub const fn texels(self) -> u32 {
        match self {
            Self::X4 => 4,
            Self::X6 => 6,
            Self::X8 => 8,
        }
    }

    /// The Vulkan format code a KTX2 container declares for it.
    pub const fn vk_format(self) -> u32 {
        match self {
            Self::X4 => VK_FORMAT_ASTC_4X4_UNORM_BLOCK,
            Self::X6 => VK_FORMAT_ASTC_6X6_UNORM_BLOCK,
            Self::X8 => VK_FORMAT_ASTC_8X8_UNORM_BLOCK,
        }
    }

    /// Bytes an image of this size occupies, or `None` when it is not a whole
    /// number of blocks.
    pub const fn encoded_len(self, width: u32, height: u32) -> Option<usize> {
        let side = self.texels();
        if width == 0 || height == 0 || !width.is_multiple_of(side) || !height.is_multiple_of(side)
        {
            return None;
        }
        Some((width / side) as usize * (height / side) as usize * ASTC_BLOCK_BYTES)
    }
}

/// The weight grid, which is 4x4 whatever the footprint.
///
/// Dual plane stores two weights per grid location, and the format caps a block
/// at 64 weights, so 4x4 is the largest square grid the second plane leaves
/// room for. It is also what makes the three footprints one encoder: the block
/// mode, the endpoint range and the weight range are identical, and only the
/// texel-to-grid mapping differs.
const GRID: u32 = 4;

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

/// Where a texel samples the weight grid, as C.2.18 computes it.
///
/// Returns the four grid indices the decoder will read and the sixteenths it
/// will weight them by. Written as the specification writes it -- integer
/// arithmetic, the same shifts -- because the encoder's fit is only as good as
/// its model of the decoder, and an approximation here is a systematic error
/// rather than a rounding one.
fn infill_taps(footprint: Footprint, s: u32, t: u32) -> [(usize, u32); 4] {
    let side = footprint.texels();
    let scale = |b: u32| (1024 + b / 2) / (b - 1);
    let cs = scale(side) * s;
    let ct = scale(side) * t;
    let gs = (cs * (GRID - 1) + 32) >> 6;
    let gt = (ct * (GRID - 1) + 32) >> 6;
    let (js, fs) = (gs >> 4, gs & 0xF);
    let (jt, ft) = (gt >> 4, gt & 0xF);

    let w11 = (fs * ft + 8) >> 4;
    let w10 = ft - w11;
    let w01 = fs - w11;
    // `(16 + w11) - fs - ft`, not the specification's `16 - fs - ft + w11`.
    // The result is the same and never negative -- the four coefficients are
    // sixteenths of one -- but evaluated left to right on unsigned integers the
    // written order underflows at `fs = ft = 15`, which is a real corner: it is
    // the texel furthest from a grid point, and every footprint larger than the
    // grid has one.
    let w00 = (16 + w11) - fs - ft;

    let v0 = (js + jt * GRID) as usize;
    [
        (v0, w00),
        (v0 + 1, w01),
        (v0 + GRID as usize, w10),
        (v0 + GRID as usize + 1, w11),
    ]
}

/// Fit a 4x4 weight grid to the per-texel weights the block wants.
///
/// The decoder spreads each grid weight over the texels that sample it, with
/// the sixteenths [`infill_taps`] returns. The fit is that operator's transpose:
/// every grid location takes the average of the texels that read it, weighted by
/// how much they read it. One step of a least-squares solve, which is as much as
/// four quantisation levels can use -- a full solve would land on the same four
/// numbers almost everywhere and cost an iteration per block at ingest.
///
/// A grid location no texel reads keeps zero and contributes nothing: the
/// decoder multiplies it by a zero coefficient.
fn fit_grid(footprint: Footprint, ideal: &[f32]) -> [u8; 16] {
    let side = footprint.texels();
    let mut sums = [0.0f32; 16];
    let mut mass = [0.0f32; 16];
    for t in 0..side {
        for s in 0..side {
            let want = ideal[(t * side + s) as usize];
            for (index, coefficient) in infill_taps(footprint, s, t) {
                if coefficient == 0 || index >= 16 {
                    continue;
                }
                let share = coefficient as f32;
                sums[index] += want * share;
                mass[index] += share;
            }
        }
    }
    let mut grid = [0u8; 16];
    for index in 0..16 {
        if mass[index] > 0.0 {
            grid[index] = quantise_unit(sums[index] / mass[index]);
        }
    }
    grid
}

/// The endpoints and weights for one tile of RGBA pixels.
///
/// Colour and alpha are solved separately because the block interpolates them
/// separately: the colour endpoints are the extremes along the tile's dominant
/// RGB axis, and alpha is a one-dimensional problem of its own.
fn solve_tile(footprint: Footprint, tile: &[[u8; 4]]) -> ([[u8; 4]; 2], [u8; 16], [u8; 16]) {
    // The dominant axis, as the bounding box's longest side. A covariance
    // eigenvector is better on tiles whose colours lie along a diagonal, and
    // measurably so only on gradients; the box is what a tile of real content
    // mostly is, and it costs no iteration.
    let mut low = [255u8; 3];
    let mut high = [0u8; 3];
    for pixel in tile {
        for channel in 0..3 {
            low[channel] = low[channel].min(pixel[channel]);
            high[channel] = high[channel].max(pixel[channel]);
        }
    }

    // Which diagonal of the box the colours actually lie along.
    //
    // A three-dimensional box has four diagonals, and taking `low -> high`
    // unconditionally is right only when every channel rises together. A tile
    // whose red rises while its blue falls lies along a different one, and
    // projecting it onto this one collapses the two channels into a single
    // muddled ramp -- the endpoints are then corners no colour in the tile is
    // near.
    //
    // Found by a test image built to be easy: red rising, blue falling, and the
    // encoder reconstructing it worse than a hard alpha edge. The sign of each
    // channel's covariance with red is what picks the diagonal, and red is the
    // reference only because some channel has to be.
    let mean = {
        let mut sums = [0.0f32; 3];
        for pixel in tile {
            for channel in 0..3 {
                sums[channel] += f32::from(pixel[channel]);
            }
        }
        sums.map(|sum| sum / tile.len() as f32)
    };
    let mut ends = [low, high];
    for channel in 1..3 {
        let covariance: f32 = tile
            .iter()
            .map(|pixel| {
                (f32::from(pixel[0]) - mean[0]) * (f32::from(pixel[channel]) - mean[channel])
            })
            .sum();
        if covariance < 0.0 {
            ends[0][channel] = high[channel];
            ends[1][channel] = low[channel];
        }
    }
    let (low, high) = (ends[0], ends[1]);

    // The projection axis, and a scale that turns a projection into 0..1.
    let axis = [
        f32::from(high[0]) - f32::from(low[0]),
        f32::from(high[1]) - f32::from(low[1]),
        f32::from(high[2]) - f32::from(low[2]),
    ];
    let length = axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2];

    // The per-texel weight each plane wants, before the grid has to represent
    // it. At the 4x4 footprint the grid holds one weight per texel and the fit
    // below is exact; at 6x6 and 8x8 it is a fit, which is the whole of what
    // those footprints trade for their smaller blocks.
    let mut colour_ideal = vec![0.0f32; tile.len()];
    if length > 0.0 {
        for (index, pixel) in tile.iter().enumerate() {
            colour_ideal[index] = ((0..3)
                .map(|channel| {
                    (f32::from(pixel[channel]) - f32::from(low[channel])) * axis[channel]
                })
                .sum::<f32>()
                / length)
                .clamp(0.0, 1.0);
        }
    }

    let alpha_low = tile.iter().map(|pixel| pixel[3]).min().unwrap_or(0);
    let alpha_high = tile.iter().map(|pixel| pixel[3]).max().unwrap_or(0);
    let mut alpha_ideal = vec![0.0f32; tile.len()];
    if alpha_high > alpha_low {
        let span = f32::from(alpha_high) - f32::from(alpha_low);
        for (index, pixel) in tile.iter().enumerate() {
            alpha_ideal[index] = (f32::from(pixel[3]) - f32::from(alpha_low)) / span;
        }
    }

    let mut colour_weights = fit_grid(footprint, &colour_ideal);
    let mut alpha_weights = fit_grid(footprint, &alpha_ideal);

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

/// What the decoder will produce for one texel of a block this encoder built.
///
/// C.2.18 for the weight, then C.2.19 for the colour: endpoints expand to
/// sixteen bits by replication, interpolate at the texel's weight, and the top
/// eight bits are the byte a `GL_RGBA8` readback returns.
///
/// This exists so the encoder can grade its own output. A footprint that is
/// smaller is only better if what it reconstructs is close enough, and "close
/// enough" is not a judgement anyone can make from the block size --
/// `crossed-gradients` is *better* at 8x8 than at 4x4 and a quarter the bytes,
/// while a sprite's alpha edge falls twenty decibels over the same change.
///
/// It is a model of the decoder, so it can be wrong. The gate compares what it
/// predicts against what the platform's decoder actually produces.
fn reconstruct_texel(
    footprint: Footprint,
    endpoints: [[u8; 4]; 2],
    colour_grid: &[u8; 16],
    alpha_grid: &[u8; 16],
    s: u32,
    t: u32,
) -> [u8; 4] {
    let interpolate = |grid: &[u8; 16]| -> u32 {
        let mut total = 0u32;
        for (index, coefficient) in infill_taps(footprint, s, t) {
            if coefficient == 0 {
                continue;
            }
            let stored = grid.get(index).copied().unwrap_or(0);
            total += u32::from(WEIGHT_LEVELS[stored as usize]) * coefficient;
        }
        (total + 8) >> 4
    };
    let colour_weight = interpolate(colour_grid);
    let alpha_weight = interpolate(alpha_grid);

    let mut out = [0u8; 4];
    for channel in 0..4 {
        let weight = if channel == 3 {
            alpha_weight
        } else {
            colour_weight
        };
        // The endpoints reach the interpolator as the levels the block stores,
        // not as the bytes the caller asked for.
        let e0 = u32::from(ENDPOINT_LEVELS[quantise_endpoint(endpoints[0][channel]) as usize]);
        let e1 = u32::from(ENDPOINT_LEVELS[quantise_endpoint(endpoints[1][channel]) as usize]);
        let c0 = (e0 << 8) | e0;
        let c1 = (e1 << 8) | e1;
        let value = (c0 * (64 - weight) + c1 * weight + 32) / 64;
        out[channel] = (value >> 8) as u8;
    }
    out
}

/// The worst per-channel error this encoder would produce for an image at a
/// given footprint.
///
/// Measured rather than guessed, on the reconstruction the decoder will perform.
pub fn worst_error(rgba: &[u8], width: u32, height: u32, footprint: Footprint) -> Option<u8> {
    footprint.encoded_len(width, height)?;
    if rgba.len() != (width as usize) * (height as usize) * 4 {
        return None;
    }
    let side = footprint.texels() as usize;
    let mut tile = vec![[0u8; 4]; side * side];
    let mut worst = 0u8;

    for block_y in 0..height as usize / side {
        for block_x in 0..width as usize / side {
            for row in 0..side {
                let y = block_y * side + row;
                let base = (y * width as usize + block_x * side) * 4;
                for column in 0..side {
                    let at = base + column * 4;
                    tile[row * side + column] =
                        [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]];
                }
            }
            let (endpoints, colour_grid, alpha_grid) = solve_tile(footprint, &tile);
            for row in 0..side {
                for column in 0..side {
                    let got = reconstruct_texel(
                        footprint,
                        endpoints,
                        &colour_grid,
                        &alpha_grid,
                        column as u32,
                        row as u32,
                    );
                    let want = tile[row * side + column];
                    for channel in 0..4 {
                        let error = got[channel].abs_diff(want[channel]);
                        worst = worst.max(error);
                    }
                }
            }
        }
    }
    Some(worst)
}

/// Encode at the smallest footprint whose reconstruction stays within `budget`.
///
/// Returns the footprint alongside the blocks, because the container has to
/// declare it.
///
/// # Why the choice is per image
///
/// The footprints are not ordered by quality. A smooth gradient reconstructs
/// *better* at 8x8 than at 4x4 -- fewer blocks means fewer endpoint
/// quantisations, and a gradient is exactly what bilinear infill represents --
/// at a quarter of the bytes. A sprite's alpha edge falls from 42 dB to 21 dB
/// over the same change, because a hard edge inside a 64-texel block is what a
/// four-by-four weight grid cannot hold. Neither is the general case, so
/// neither can be the default.
///
/// 6x6 is offered and will almost never be chosen: `encoded_len` refuses an
/// image that is not a whole number of blocks, and no power of two is a
/// multiple of six. It is here because a texture atlas that happens to be sized
/// in multiples of six should not be denied it.
pub fn encode_astc_within(
    rgba: &[u8],
    width: u32,
    height: u32,
    budget: u8,
) -> Result<(Vec<u8>, Footprint), AstcError> {
    // Largest block first: the first that fits the budget is the fewest bytes.
    for footprint in [Footprint::X8, Footprint::X6, Footprint::X4] {
        if footprint.encoded_len(width, height).is_none() {
            continue;
        }
        match worst_error(rgba, width, height, footprint) {
            Some(error) if error <= budget => {
                return encode_astc(rgba, width, height, footprint)
                    .map(|blocks| (blocks, footprint));
            }
            _ => continue,
        }
    }
    encode_astc(rgba, width, height, Footprint::X4).map(|blocks| (blocks, Footprint::X4))
}

/// Encode an RGBA8 image as ASTC LDR blocks, in raster block order.
pub fn encode_astc(
    rgba: &[u8],
    width: u32,
    height: u32,
    footprint: Footprint,
) -> Result<Vec<u8>, AstcError> {
    let expected = (width as usize) * (height as usize) * 4;
    if rgba.len() != expected {
        return Err(AstcError::BadInputLength {
            expected,
            actual: rgba.len(),
        });
    }
    let capacity = footprint
        .encoded_len(width, height)
        .ok_or(AstcError::UnalignedDimensions { width, height })?;

    let side = footprint.texels() as usize;
    let blocks_x = width as usize / side;
    let blocks_y = height as usize / side;
    let mut out = Vec::with_capacity(capacity);
    let mut tile = vec![[0u8; 4]; side * side];

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            for row in 0..side {
                let y = block_y * side + row;
                let base = (y * width as usize + block_x * side) * 4;
                for column in 0..side {
                    let at = base + column * 4;
                    tile[row * side + column] =
                        [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]];
                }
            }
            let (endpoints, colour_weights, alpha_weights) = solve_tile(footprint, &tile);
            out.extend_from_slice(&assemble_block(endpoints, colour_weights, alpha_weights));
        }
    }
    Ok(out)
}

/// The 4x4 footprint, which is what the ingest path takes today.
pub fn encode_astc_4x4(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, AstcError> {
    encode_astc(rgba, width, height, Footprint::X4)
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

#[cfg(test)]
mod infill_tests {
    use super::*;

    /// The four interpolation coefficients are sixteenths of one, at every
    /// texel of every footprint.
    ///
    /// This is the invariant the decoder's `(p00*w00 + ... + 8) >> 4` relies on:
    /// coefficients summing to anything but sixteen scale the weight, which
    /// scales the colour. It also walks the corner that made the specification's
    /// own expression underflow when evaluated on unsigned integers.
    #[test]
    fn the_infill_coefficients_are_sixteenths_of_one() {
        for footprint in [Footprint::X4, Footprint::X6, Footprint::X8] {
            let side = footprint.texels();
            for t in 0..side {
                for s in 0..side {
                    let taps = infill_taps(footprint, s, t);
                    let total: u32 = taps.iter().map(|(_, weight)| *weight).sum();
                    assert_eq!(
                        total, 16,
                        "{side}x{side} texel ({s},{t}) interpolates with {total}/16"
                    );
                }
            }
        }
    }

    /// At the 4x4 footprint the grid is the texel array, so the mapping is the
    /// identity: each texel reads exactly one grid location, at full weight.
    ///
    /// Which is why the 4x4 path needs no fit, and why a fault in the infill
    /// shows up only in the larger footprints.
    #[test]
    fn the_four_by_four_footprint_reads_one_grid_location_per_texel() {
        for t in 0..4 {
            for s in 0..4 {
                let taps = infill_taps(Footprint::X4, s, t);
                let full: Vec<_> = taps.iter().filter(|(_, weight)| *weight == 16).collect();
                assert_eq!(full.len(), 1, "texel ({s},{t}) does not read one location");
                assert_eq!(
                    full[0].0,
                    (s + t * GRID) as usize,
                    "texel ({s},{t}) reads the wrong location"
                );
            }
        }
    }

    /// Every grid location the larger footprints can name is one the block
    /// stores.
    ///
    /// The specification lets the bilinear read run off the end of a row when
    /// the fractional part is zero, because the coefficient is then zero too.
    /// The fit must skip those rather than write past its array, and this is
    /// what says the ones with a non-zero coefficient are all in range.
    #[test]
    fn every_grid_location_with_weight_is_one_the_block_holds() {
        for footprint in [Footprint::X4, Footprint::X6, Footprint::X8] {
            let side = footprint.texels();
            for t in 0..side {
                for s in 0..side {
                    for (index, weight) in infill_taps(footprint, s, t) {
                        if weight > 0 {
                            assert!(
                                index < 16,
                                "{side}x{side} texel ({s},{t}) reads grid slot {index} \
                                 with weight {weight}"
                            );
                        }
                    }
                }
            }
        }
    }
}
