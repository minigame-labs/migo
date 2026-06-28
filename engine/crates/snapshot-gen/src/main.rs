//! V8 startup snapshot generator for the Migo JS runtime.
//!
//! Generates a serialized V8 heap snapshot containing all pre-parsed and
//! pre-compiled extension JS modules.  Loading from this snapshot at runtime
//! eliminates ~150–300 ms of JS parsing/compilation on cold start.
//!
//! # Usage
//!
//! Snapshots are platform-bound (OS + CPU arch), so this generator must be
//! cross-compiled to a target Android ABI and run on that ABI's emulator
//! (x86_64) or device (arm64):
//!
//! ```bash
//! # Cross-compile to the target ABI (see crates/js-runtime memory notes for
//! # the exact RUSTY_V8_ARCHIVE / cargo-ndk invocation), push to the device,
//! # then run with MIGO_SNAPSHOT_OUT and `adb pull` the result into
//! # crates/js-runtime/snapshots/SNAPSHOT-<arch>.bin.
//! ```
//!
//! When run without `MIGO_SNAPSHOT_OUT`, it writes to
//! `crates/js-runtime/snapshots/SNAPSHOT-<arch>.bin` (arch = the ABI this
//! binary was compiled for). `js-runtime/build.rs` embeds the matching file
//! for android targets at compile time.
//!
//! # When to regenerate
//!
//! The snapshot is tied to the exact V8 version and extension set.  Regenerate
//! whenever any of the following change:
//!
//! - deno_core / V8 version
//! - Extension JS source files (any modification)
//! - Extension registration order
//! - Op signatures (number of ops, their names)
//!
//! In the build pipeline, always regenerate the snapshot before compiling the
//! final release artifact.

use std::path::PathBuf;

fn main() {
    // Initialize V8 platform (required before any V8 operations).
    deno_core::JsRuntime::init_platform(None);

    let extensions = js_runtime::snapshot::lazy_extensions();

    println!(
        "Creating V8 snapshot with {} extensions...",
        extensions.len()
    );
    for ext in &extensions {
        println!("  - {}", ext.name);
    }

    let output = deno_core::snapshot::create_snapshot(
        deno_core::snapshot::CreateSnapshotOptions {
            cargo_manifest_dir: env!("CARGO_MANIFEST_DIR"),
            startup_snapshot: None,
            skip_op_registration: false,
            extensions,
            extension_transpiler: None,
            with_runtime_cb: None,
        },
        None, // no warmup script
    )
    .expect("Failed to create V8 snapshot");

    // Output path. MIGO_SNAPSHOT_OUT overrides the default — required when the
    // generator runs cross-compiled on a device/emulator (V8 startup snapshots
    // are platform-bound, so each ABI's snapshot must be produced by the SAME
    // android V8 the .so links), where the host CARGO_MANIFEST_DIR doesn't
    // exist: write to e.g. /data/local/tmp/SNAPSHOT.bin then `adb pull` it into
    // `crates/js-runtime/snapshots/SNAPSHOT-<arch>.bin`.
    //
    // The default path is per-arch (`std::env::consts::ARCH` is the arch this
    // generator was compiled for), matching what `js-runtime/build.rs` selects.
    let snapshot_path: PathBuf = match std::env::var_os("MIGO_SNAPSHOT_OUT") {
        Some(p) => PathBuf::from(p),
        None => [
            env!("CARGO_MANIFEST_DIR"),
            "..",
            "js-runtime",
            "snapshots",
            &format!("SNAPSHOT-{}.bin", std::env::consts::ARCH),
        ]
        .iter()
        .collect(),
    };

    if let Some(dir) = snapshot_path.parent() {
        std::fs::create_dir_all(dir).ok();
    }

    let snapshot_data = &*output.output;
    std::fs::write(&snapshot_path, snapshot_data).expect("Failed to write SNAPSHOT.bin");

    println!(
        "V8 snapshot written to {} ({} bytes, {:.1} KB)",
        snapshot_path.display(),
        snapshot_data.len(),
        snapshot_data.len() as f64 / 1024.0,
    );

    // Print files that were loaded during snapshot creation.
    // Useful for CI cache invalidation.
    if !output.files_loaded_during_snapshot.is_empty() {
        println!("\nFiles loaded during snapshot creation:");
        for f in &output.files_loaded_during_snapshot {
            println!("  {}", f.display());
        }
    }
}
