mod common;

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use artifact_manifest::{PackageIndex, ReleaseAttestation, SliceManifest, V8ComponentManifest};
use common::{android_manifest, android_v8_component};

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "migo-artifact-manifest-cli-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create temporary test directory");
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

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_migo-artifact-manifest")
}

fn assert_success(output: Output) -> Output {
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn write_json(path: &Path, value: &impl serde::Serialize) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

#[test]
fn seal_slice_then_verify_slice() {
    let directory = TestDir::new("slice");
    let input = directory.path().join("slice-input.json");
    let output = directory.path().join("slice.json");
    let mut manifest = android_manifest("aarch64");
    manifest.artifact_id.clear();
    write_json(&input, &manifest);

    assert_success(
        Command::new(binary())
            .arg("seal-slice")
            .arg(&input)
            .arg(&output)
            .output()
            .unwrap(),
    );
    let sealed: SliceManifest = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(sealed.artifact_id.len(), 64);

    assert_success(
        Command::new(binary())
            .arg("verify-slice")
            .arg(&output)
            .output()
            .unwrap(),
    );
}

#[test]
fn verify_slice_returns_failure_for_tampered_identity() {
    let directory = TestDir::new("slice-tamper");
    let manifest_path = directory.path().join("slice.json");
    let mut manifest = android_manifest("aarch64");
    manifest.toolchain.sdk = "tampered sdk".to_string();
    write_json(&manifest_path, &manifest);

    let output = Command::new(binary())
        .arg("verify-slice")
        .arg(&manifest_path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("artifact_id mismatch"));
}

#[test]
fn build_index_then_verify_embedded_slices() {
    let directory = TestDir::new("index");
    let package_root = directory.path().join("package");
    let slice_root = package_root.join("assets/migo/artifacts/slices");
    fs::create_dir_all(&slice_root).unwrap();
    let arm_path = slice_root.join("arm64-v8a.json");
    let x64_path = slice_root.join("x86_64.json");
    write_json(&arm_path, &android_manifest("aarch64"));
    write_json(&x64_path, &android_manifest("x86_64"));
    let index_path = directory.path().join("package-index.json");
    let arm_source = format!(
        "assets/migo/artifacts/slices/arm64-v8a.json={}",
        arm_path.display()
    );
    let x64_source = format!(
        "assets/migo/artifacts/slices/x86_64.json={}",
        x64_path.display()
    );

    assert_success(
        Command::new(binary())
            .arg("build-index")
            .arg("full")
            .arg(&index_path)
            .arg(x64_source)
            .arg(arm_source)
            .output()
            .unwrap(),
    );
    let index: PackageIndex = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    assert_eq!(index.slices.len(), 2);
    assert_eq!(index.slices[0].arch, "aarch64");

    assert_success(
        Command::new(binary())
            .arg("verify-index")
            .arg(&index_path)
            .arg(&package_root)
            .output()
            .unwrap(),
    );
}

#[test]
fn attest_then_verify_final_package() {
    let directory = TestDir::new("attest");
    let package_path = directory.path().join("migo-full.aar");
    let index_path = directory.path().join("package-index.json");
    let attestation_path = directory.path().join("migo-full.aar.attestation.json");
    fs::write(&package_path, b"fixture-aar").unwrap();
    fs::write(&index_path, b"fixture-index").unwrap();

    assert_success(
        Command::new(binary())
            .arg("attest")
            .arg(&package_path)
            .arg(&index_path)
            .arg(&attestation_path)
            .output()
            .unwrap(),
    );
    let attestation: ReleaseAttestation =
        serde_json::from_slice(&fs::read(&attestation_path).unwrap()).unwrap();
    assert_eq!(attestation.package_file, "migo-full.aar");

    assert_success(
        Command::new(binary())
            .arg("verify-attestation")
            .arg(&attestation_path)
            .arg(&package_path)
            .arg(&index_path)
            .output()
            .unwrap(),
    );

    fs::write(&package_path, b"tamperd-aar").unwrap();
    let output = Command::new(binary())
        .arg("verify-attestation")
        .arg(&attestation_path)
        .arg(&package_path)
        .arg(&index_path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("package_sha256"));
}

#[test]
fn seal_v8_component_then_verify_its_files() {
    let directory = TestDir::new("v8-component");
    let archive = directory.path().join("librusty_v8.a");
    let binding = directory.path().join("src_binding.rs");
    let input = directory.path().join("component-input.json");
    let output = directory.path().join("component-manifest.json");
    fs::write(&archive, b"fixture archive").unwrap();
    fs::write(&binding, b"fixture binding").unwrap();
    let mut component = android_v8_component("aarch64");
    component.component_id.clear();
    component.hashes.archive = artifact_manifest::sha256_file(&archive).unwrap();
    component.hashes.rust_binding = artifact_manifest::sha256_file(&binding).unwrap();
    write_json(&input, &component);

    assert_success(
        Command::new(binary())
            .arg("seal-v8-component")
            .arg(&input)
            .arg(&output)
            .output()
            .unwrap(),
    );
    let sealed: V8ComponentManifest = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(sealed.component_id.len(), 64);

    assert_success(
        Command::new(binary())
            .arg("verify-v8-component")
            .arg(&output)
            .arg(&archive)
            .arg(&binding)
            .output()
            .unwrap(),
    );
}
