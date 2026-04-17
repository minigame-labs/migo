//! Shared test infrastructure for graphics-crate integration tests.
//!
//! This module is included by every test file via `#[path]` rather than
//! `mod common;` because Cargo's integration-test layout does not expose a
//! shared library crate between `tests/*.rs` files.  To add a new helper,
//! put it here and reference it from the test's `#[path = "common/mod.rs"]`
//! import block.
//!
//! Three pillars live here:
//!
//!   1. [`harness`] — builds an `SkSurface` suitable for running Canvas2D
//!      command tapes against.  Two flavours:
//!        * raster CPU surface (no GL context, fastest; used by the
//!          majority of golden tests in Phase 4)
//!        * GPU surface via an offscreen EGL pbuffer (Phase 6+ when the
//!          GL-specific paths such as `glTexStorage2D` / AHB need coverage)
//!   2. [`pixel_diff`] — load a golden PNG from `tests/goldens/`, compare
//!      against rendered pixels with an L∞ tolerance, and (on failure) dump
//!      `<name>_actual.png` + `<name>_diff.png` next to the golden to make
//!      the mismatch visually inspectable.
//!   3. [`golden`] — dry-run / regenerate helper.  Setting the env var
//!      `MIGO_REGENERATE_GOLDENS=1` rewrites any mismatching golden from
//!      the current render output instead of failing.  Use it after an
//!      intentional rendering change is reviewed.

#![allow(dead_code)]

pub mod golden;
pub mod harness;
pub mod pixel_diff;

/// Absolute path of the `tests/` directory at compile time, for locating
/// goldens and fixtures regardless of cwd.
pub const TESTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");
