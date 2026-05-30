//! Golden-image assertion and regeneration workflow.
//!
//! Tests call [`assert_matches_golden`] with a stable test name and the
//! rendered RGBA buffer.  The first invocation for a new name fails and
//! writes the rendered image as `tests/goldens/<name>.png` (so that a
//! subsequent run bootstraps the golden).  To regenerate an *existing*
//! golden after a rendering change, set the environment variable
//! `MIGO_REGENERATE_GOLDENS=1` — the comparison will be skipped and the
//! golden file overwritten instead of failing.
//!
//! Diff dumps (`<name>_actual.png`, `<name>_diff.png`) are emitted on
//! failure under `target/test-output/<name>/` so multiple failures don't
//! clobber each other.

use std::path::{Path, PathBuf};

use super::TESTS_DIR;
use super::pixel_diff::{
    PixelDiff, compare_tolerant, diff_image, dump_diff_summary, load_png_rgba8, save_png_rgba8,
};

pub struct GoldenCfg {
    pub tolerance: u8,
    pub max_failing_pixels: u32,
}

impl Default for GoldenCfg {
    /// Tight defaults: exact match with up to `0` failing pixels.  Use
    /// tolerant comparison for any test that involves anti-aliased edges.
    fn default() -> Self {
        Self {
            tolerance: 0,
            max_failing_pixels: 0,
        }
    }
}

impl GoldenCfg {
    /// Convenience: tolerance-2 up to 1% failing pixels — a sensible
    /// baseline for AA-heavy tests (strokes, text, shadows) where driver
    /// variance can push a handful of edge pixels past exact equality.
    pub fn loose_aa(width: u32, height: u32) -> Self {
        Self {
            tolerance: 2,
            max_failing_pixels: width * height / 100,
        }
    }
}

pub fn goldens_dir() -> PathBuf {
    Path::new(TESTS_DIR).join("goldens")
}

pub fn debug_out_dir(test_name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
        .parent()
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
        .join("target/test-output")
        .join(test_name)
}

fn regenerate_requested() -> bool {
    std::env::var_os("MIGO_REGENERATE_GOLDENS").is_some()
}

/// Compare `rgba` against the stored golden `<test_name>.png` with the
/// given [`GoldenCfg`].  See module docs for the bootstrap + regenerate
/// workflow.
pub fn assert_matches_golden(test_name: &str, w: u32, h: u32, rgba: &[u8], cfg: GoldenCfg) {
    assert!(
        !test_name.contains(|c: char| c == '/' || c == '\\' || c == '.'),
        "test_name must be a stable slug (no path separators, no dots): {test_name:?}",
    );
    assert_eq!(rgba.len(), (w * h * 4) as usize);

    let golden_path = goldens_dir().join(format!("{test_name}.png"));

    // ---- regenerate mode ---------------------------------------------
    if regenerate_requested() {
        save_png_rgba8(&golden_path, w, h, rgba);
        eprintln!(
            "[MIGO_REGENERATE_GOLDENS] wrote {} ({}×{} bytes={})",
            golden_path.display(),
            w,
            h,
            rgba.len(),
        );
        return;
    }

    // ---- bootstrap mode (golden missing) -----------------------------
    if !golden_path.exists() {
        save_png_rgba8(&golden_path, w, h, rgba);
        panic!(
            "golden {} did not exist; created it from current render.  \
             Review visually and commit; re-run the test to verify.",
            golden_path.display(),
        );
    }

    // ---- compare against existing golden -----------------------------
    let (gw, gh, expected) = load_png_rgba8(&golden_path);
    assert_eq!(
        (gw, gh),
        (w, h),
        "golden {} dimensions {gw}×{gh} disagree with rendered {w}×{h}",
        golden_path.display(),
    );

    let diff: PixelDiff = compare_tolerant(w, h, rgba, &expected, cfg.tolerance);
    if diff.failing_pixels <= cfg.max_failing_pixels {
        return;
    }

    // ---- failure: dump diff artifacts --------------------------------
    let out_dir = debug_out_dir(test_name);
    let _ = std::fs::create_dir_all(&out_dir);
    let actual_png = out_dir.join(format!("{test_name}_actual.png"));
    let diff_png = out_dir.join(format!("{test_name}_diff.png"));
    save_png_rgba8(&actual_png, w, h, rgba);
    let diff_buf = diff_image(w, h, rgba, &expected, cfg.tolerance);
    save_png_rgba8(&diff_png, w, h, &diff_buf);
    dump_diff_summary(test_name, &diff);

    panic!(
        "golden mismatch for `{test_name}` (tolerance={}, allowed={}, got={} failing pixels, \
         max_abs_delta={}).  See\n    actual: {}\n    diff:   {}\n    golden: {}\n\
         Re-run with MIGO_REGENERATE_GOLDENS=1 to accept the new rendering.",
        cfg.tolerance,
        cfg.max_failing_pixels,
        diff.failing_pixels,
        diff.max_abs_delta,
        actual_png.display(),
        diff_png.display(),
        golden_path.display(),
    );
}
