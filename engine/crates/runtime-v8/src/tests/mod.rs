//! Unit tests for the V8 runtime backend.
//!
//! Grouped here rather than sitting beside the modules they exercise: they were
//! seven `tests_*.rs` files at the crate root, interleaved with production
//! sources, and three of them were named after internal ticket ids whose
//! meaning lived only in the performance audit document.

mod binary_helper;
mod global_surface;
mod install_receipt;
mod prelude;
mod snapshot_fingerprint;
mod timers;
mod v8_limits;
