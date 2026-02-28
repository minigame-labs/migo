//! V8 startup snapshot generator for the Migo JS runtime.
//!
//! Generates a serialized V8 heap snapshot containing all pre-parsed and
//! pre-compiled extension JS modules.  Loading from this snapshot at runtime
//! eliminates ~150–300 ms of JS parsing/compilation on cold start.
//!
//! # Usage
//!
//! ```bash
//! # From engine/ directory:
//! cargo run -p migo-snapshot-gen
//! ```
//!
//! This writes `crates/js-runtime/SNAPSHOT.bin`.  The snapshot is always
//! embedded at compile time — release builds fail without it.
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

    println!("Creating V8 snapshot with {} extensions...", extensions.len());
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

    // Write raw snapshot to js-runtime crate directory.
    let snapshot_path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        "js-runtime",
        "SNAPSHOT.bin",
    ]
    .iter()
    .collect();

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
