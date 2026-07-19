//! Compiles the C host example and links the window-system libraries it needs.
//!
//! The C source lives outside the crate (`examples/c-host/main.c`) because it is
//! documentation as much as it is a test: a host author should be able to read
//! it without knowing anything about cargo.

use std::path::PathBuf;

fn main() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repo root");
    let source = repo_root.join("examples/c-host/main.c");
    let include = repo_root.join("include");

    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed={}", include.display());

    cc::Build::new()
        .file(&source)
        .include(&include)
        .std("c11")
        .warnings(true)
        .compile("c_host_main");

    // The example owns its window, so it — not the engine — needs Xlib.
    println!("cargo:rustc-link-lib=X11");
    // Skia is built with `skia_use_gl`, so its objects reference `gl*` directly
    // and the final binary has to supply libGL; the Rust player links it for the
    // same reason.
    println!("cargo:rustc-link-lib=GL");
    println!("cargo:rustc-link-lib=EGL");
}
