//! Build script for js-runtime crate.
//!
//! Sets the `migo_has_snapshot` cfg flag when `SNAPSHOT.bin` exists alongside
//! this crate's source files.  This allows `snapshot.rs` to conditionally
//! `include_bytes!("SNAPSHOT.bin")` at compile time.

use std::path::Path;

fn main() {
    // Declare custom cfg for check-cfg lint.
    println!("cargo::rustc-check-cfg=cfg(migo_has_snapshot)");

    // Re-run if the snapshot file is added, removed, or regenerated.
    println!("cargo:rerun-if-changed=SNAPSHOT.bin");

    let profile = std::env::var("PROFILE").unwrap_or_default();
    let snapshot_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("SNAPSHOT.bin");
    if snapshot_path.exists() {
        println!("cargo:rustc-cfg=migo_has_snapshot");
        println!(
            "cargo:warning=V8 snapshot found ({} bytes), embedding into binary",
            std::fs::metadata(&snapshot_path)
                .map(|m| m.len())
                .unwrap_or(0)
        );
    } else if profile == "release" {
        println!("cargo:warning=No V8 snapshot found — release builds will fail to compile");
    }
}
