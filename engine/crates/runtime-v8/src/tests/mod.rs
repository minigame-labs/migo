//! Unit tests for the V8 runtime backend.
//!
//! Grouped here rather than sitting beside the modules they exercise: they were
//! seven `tests_*.rs` files at the crate root, interleaved with production
//! sources, and three of them were named after internal ticket ids whose
//! meaning lived only in the performance audit document.

mod ad_reward_integrity;
mod binary_helper;
mod canvas_follows_surface;
mod global_surface;
mod host_bridge_dispatch;
mod install_receipt;
mod permission_reporting;
mod permission_revocation;
mod prelude;
mod published_namespace_isolation;
mod snapshot_fingerprint;
mod storage_isolation;
mod timers;
mod two_session_identity;
mod v8_limits;
