//! RGBA pixel comparator with tolerance + PNG round-trip for goldens.
//!
//! Two levels of strictness:
//!   * [`compare_exact`] — every channel must match bit-for-bit.  Used for
//!     simple shapes with flat colours (no AA).
//!   * [`compare_tolerant`] — L∞ tolerance per channel.  Required for any
//!     AA-heavy image (strokes, text, shadows) because GL drivers diverge
//!     at the sub-pixel level.
//!
//! Both return a [`PixelDiff`] summary on failure so callers can turn it
//! into a rich error message (max delta, failing pixel count, first
//! failing coord).

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

use png::{BitDepth, ColorType, Decoder, Encoder};

/// Summary of a pixel-diff failure.
#[derive(Debug)]
pub struct PixelDiff {
    pub width: u32,
    pub height: u32,
    /// Count of pixels where at least one channel exceeded the tolerance.
    pub failing_pixels: u32,
    /// Largest absolute channel delta observed across all pixels.
    pub max_abs_delta: u8,
    /// First `(x, y)` that failed (row-major scan order), for error msgs.
    pub first_failure: Option<(u32, u32, [u8; 4], [u8; 4])>,
}

impl PixelDiff {
    pub fn is_ok(&self) -> bool {
        self.failing_pixels == 0
    }
}

/// Load a PNG file and return `(width, height, rgba_bytes)`.
///
/// Panics if the PNG does not decode as 8-bit RGBA.  Goldens are always
/// authored in that format so a mismatch is a programmer error.
pub fn load_png_rgba8(path: &Path) -> (u32, u32, Vec<u8>) {
    let file = File::open(path)
        .unwrap_or_else(|e| panic!("failed to open golden {path:?}: {e}"));
    let decoder = Decoder::new(BufReader::new(file));
    let mut reader = decoder
        .read_info()
        .unwrap_or_else(|e| panic!("failed to read PNG header {path:?}: {e}"));
    let info = reader.info();
    assert_eq!(
        info.color_type,
        ColorType::Rgba,
        "golden {path:?} must be 8-bit RGBA (got {:?})",
        info.color_type,
    );
    assert_eq!(
        info.bit_depth,
        BitDepth::Eight,
        "golden {path:?} must be 8-bit (got {:?})",
        info.bit_depth,
    );
    let (w, h) = (info.width, info.height);
    let mut buf = vec![0u8; reader.output_buffer_size()];
    reader
        .next_frame(&mut buf)
        .unwrap_or_else(|e| panic!("failed to decode {path:?}: {e}"));
    (w, h, buf)
}

/// Write an 8-bit RGBA buffer to `path` as a PNG.  Intermediate directories
/// are created on-demand.  Overwrites any existing file.
pub fn save_png_rgba8(path: &Path, w: u32, h: u32, rgba: &[u8]) {
    assert_eq!(rgba.len(), (w * h * 4) as usize);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("mkdir -p {parent:?}: {e}"));
    }
    let file = File::create(path)
        .unwrap_or_else(|e| panic!("create {path:?}: {e}"));
    let mut encoder = Encoder::new(BufWriter::new(file), w, h);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .unwrap_or_else(|e| panic!("png header {path:?}: {e}"));
    writer
        .write_image_data(rgba)
        .unwrap_or_else(|e| panic!("png body {path:?}: {e}"));
}

/// Exact comparison (tolerance = 0) of two RGBA buffers of identical shape.
pub fn compare_exact(
    w: u32,
    h: u32,
    actual: &[u8],
    expected: &[u8],
) -> PixelDiff {
    compare_tolerant(w, h, actual, expected, 0)
}

/// L∞-tolerance comparison: a pixel is considered "failing" when at least
/// one of its four channels diverges from the expected value by more than
/// `tolerance` (absolute value, 0..=255).
pub fn compare_tolerant(
    w: u32,
    h: u32,
    actual: &[u8],
    expected: &[u8],
    tolerance: u8,
) -> PixelDiff {
    assert_eq!(
        actual.len(),
        expected.len(),
        "actual/expected size mismatch: {} vs {} bytes",
        actual.len(),
        expected.len(),
    );
    assert_eq!(
        actual.len(),
        (w * h * 4) as usize,
        "buffer size does not match width×height×4",
    );

    let mut failing = 0u32;
    let mut max_delta = 0u8;
    let mut first: Option<(u32, u32, [u8; 4], [u8; 4])> = None;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let a = [actual[i], actual[i + 1], actual[i + 2], actual[i + 3]];
            let e = [
                expected[i],
                expected[i + 1],
                expected[i + 2],
                expected[i + 3],
            ];
            let mut px_failed = false;
            for c in 0..4 {
                let d = a[c].abs_diff(e[c]);
                if d > max_delta {
                    max_delta = d;
                }
                if d > tolerance {
                    px_failed = true;
                }
            }
            if px_failed {
                failing += 1;
                if first.is_none() {
                    first = Some((x, y, a, e));
                }
            }
        }
    }

    PixelDiff {
        width: w,
        height: h,
        failing_pixels: failing,
        max_abs_delta: max_delta,
        first_failure: first,
    }
}

/// Build a synthetic "diff" image that highlights divergent pixels in red
/// on a 50%-grey background, for human inspection.
pub fn diff_image(
    w: u32,
    h: u32,
    actual: &[u8],
    expected: &[u8],
    tolerance: u8,
) -> Vec<u8> {
    let mut out = vec![128u8; (w * h * 4) as usize];
    for i in (0..out.len()).step_by(4) {
        out[i + 3] = 255;
        let mismatch = (0..4).any(|c| actual[i + c].abs_diff(expected[i + c]) > tolerance);
        if mismatch {
            out[i] = 255;
            out[i + 1] = 0;
            out[i + 2] = 0;
        }
    }
    out
}

/// Helper used by test bodies: rich-format the diff to stderr before
/// asserting.  Makes CI log excerpts self-contained.
pub fn dump_diff_summary(name: &str, diff: &PixelDiff) {
    let mut s = format!(
        "pixel-diff [{}]: {}×{}, failing={}, max_abs_delta={}",
        name, diff.width, diff.height, diff.failing_pixels, diff.max_abs_delta,
    );
    if let Some((x, y, a, e)) = diff.first_failure {
        use std::fmt::Write as _;
        let _ = write!(
            &mut s,
            ", first_mismatch=(x={x}, y={y}, actual={a:?}, expected={e:?})",
        );
    }
    // Writing to stderr (eprintln!) is fine but wrapping it here ensures a
    // terminating newline even when callers forget one.
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(&mut stderr, "{s}");
}
