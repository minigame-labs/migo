mod common;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use artifact_manifest::{
    seal_v8_component_manifest, sha256_file, validate_v8_component_manifest,
    verify_v8_component_files,
};
use common::android_v8_component;

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "migo-v8-component-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn accepts_verified_android_v8_component_for_each_abi() {
    validate_v8_component_manifest(&android_v8_component("aarch64")).unwrap();
    validate_v8_component_manifest(&android_v8_component("x86_64")).unwrap();
}

#[test]
fn rejects_component_identity_tampering() {
    let mut component = android_v8_component("aarch64");
    component.toolchain.sdk = "different NDK".to_string();

    let error = validate_v8_component_manifest(&component).unwrap_err();
    assert!(error.to_string().contains("component_id mismatch"));
}

#[test]
fn rejects_component_source_revision_that_disagrees_with_rusty_v8() {
    let mut component = android_v8_component("aarch64");
    component.provenance.source_revision = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
    seal_v8_component_manifest(&mut component).unwrap();

    let error = validate_v8_component_manifest(&component).unwrap_err();
    assert!(error.to_string().contains("source_revision"));
}

#[test]
fn verifies_archive_and_binding_bytes() {
    let directory = TestDir::new("files");
    let archive = directory.path().join("librusty_v8.a");
    let binding = directory.path().join("src_binding.rs");
    fs::write(&archive, b"fixture archive").unwrap();
    fs::write(&binding, b"fixture binding").unwrap();
    let mut component = android_v8_component("aarch64");
    component.hashes.archive = sha256_file(&archive).unwrap();
    component.hashes.rust_binding = sha256_file(&binding).unwrap();
    seal_v8_component_manifest(&mut component).unwrap();

    verify_v8_component_files(&component, &archive, &binding).unwrap();
    fs::write(&binding, b"tampered binding").unwrap();
    let error = verify_v8_component_files(&component, &archive, &binding).unwrap_err();
    assert!(error.to_string().contains("rust_binding"));
}
