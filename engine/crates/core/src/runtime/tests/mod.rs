//! Source-level contract guards for the host runtime.
//!
//! They read the production modules with `include_str!` and assert on the
//! text, so they hold without linking the host EGL/Skia stack. Grouped here
//! rather than beside the modules they check: they are one body of rules
//! about the same thread, and splitting them hid the fact that they overlap.

mod blocking_pool_policy;
mod session_teardown_caches;
mod startup_ordering;
mod thread_wiring;
