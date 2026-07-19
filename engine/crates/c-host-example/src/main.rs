//! Entry point comes from C.
//!
//! `main` is defined in `examples/c-host/main.c`, which build.rs compiles and
//! links in. `#![no_main]` keeps Rust from emitting one of its own, so the
//! resulting binary is a C program that happens to be linked by cargo — which
//! is the point: the C side sees nothing but the public ABI, while cargo
//! resolves the engine's native dependencies correctly.
//!
//! The `capi` dependency is what pulls the `migo_*` symbols into the link.

#![no_main]

// Referencing the crate keeps the linker from dropping it: nothing in Rust
// calls into it, the C side does. (`capi`'s library is named `migo_capi`.)
extern crate migo_capi as _;
