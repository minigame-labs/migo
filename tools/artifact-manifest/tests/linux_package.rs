//! The Linux GNU package manifest contract.
//!
//! The failures worth catching here are not malformed JSON -- serde rejects
//! that -- but manifests that parse cleanly and describe a package that cannot
//! run: a V8 built for another target, a snapshot for another architecture, a
//! loader floor quietly raised past what consumers were promised.

use artifact_manifest::{
    LinuxPackageManifest, PackageSnapshotIdentity, validate_linux_package_manifest,
};

fn shipped_manifest() -> LinuxPackageManifest {
    serde_json::from_str(SHIPPED).expect("the shipped manifest shape must parse")
}

/// A copy of what scripts/gen-linux-package-metadata.py emits. If the generator
/// changes shape, this stops parsing -- which is the intended failure, since the
/// validator and the generator have to agree on one document.
const SHIPPED: &str = r#"{
  "schema": "migo-linux-package-v1",
  "version": "0.1.0",
  "target": "x86_64-unknown-linux-gnu",
  "os": "linux",
  "abi": "gnu",
  "arch": "x86_64",
  "cpu_baseline": "x86-64-v1",
  "required_cpu_features": ["cmov", "sse2"],
  "glibc_floor": "2.31",
  "glibcxx_floor": "3.4.28",
  "sysroot": "/build/linux/debian_bullseye_amd64-sysroot",
  "dynamic_dependencies": ["libEGL.so.1", "libc.so.6", "libstdc++.so.6"],
  "snapshot_policy": "none",
  "snapshots": [],
  "v8": {
    "schema": "migo-v8-component-v1",
    "target": "x86_64-unknown-linux-gnu",
    "rusty_v8_revision": "0b8cfc5ae9d2507031076df2acdf61b0742a4c4e",
    "v8_version": "14.5.201.2"
  },
  "artifacts": { "libmigo.a": 1024, "libmigo.so.0.1.0": 2048 }
}"#;

#[test]
fn the_shipped_manifest_shape_is_valid() {
    validate_linux_package_manifest(&shipped_manifest()).expect("shipped manifest must validate");
}

#[test]
fn a_v8_built_for_another_target_cannot_be_shipped_in_this_package() {
    let mut manifest = shipped_manifest();
    manifest.v8.target = "aarch64-linux-android".to_string();
    let error = validate_linux_package_manifest(&manifest)
        .expect_err("an Android V8 in a Linux package must be rejected");
    assert!(
        error.to_string().contains("does not match package target"),
        "error should name the mismatch, got: {error}"
    );
}

#[test]
fn linux_is_a_kernel_not_an_abi_so_the_abi_is_pinned_separately() {
    // Android and OpenHarmony are Linux kernels too. A manifest claiming os
    // linux says nothing about whether the package loads there.
    let mut manifest = shipped_manifest();
    manifest.abi = "android".to_string();
    assert!(
        validate_linux_package_manifest(&manifest).is_err(),
        "a non-gnu ABI must not validate as the desktop Linux slice"
    );
}

#[test]
fn the_loader_floor_cannot_be_raised_without_changing_the_promise() {
    for (field, value) in [("glibc", "2.38"), ("glibcxx", "3.4.32")] {
        let mut manifest = shipped_manifest();
        if field == "glibc" {
            manifest.glibc_floor = value.to_string();
        } else {
            manifest.glibcxx_floor = value.to_string();
        }
        assert!(
            validate_linux_package_manifest(&manifest).is_err(),
            "{field} floor {value} is above what consumers were promised"
        );
    }
}

#[test]
fn shipping_no_snapshot_must_be_stated_and_stay_consistent() {
    // "none" with a snapshot listed, and "embedded" with none, are both the
    // same underlying bug: the statement and the content disagreeing.
    let mut manifest = shipped_manifest();
    manifest.snapshots = vec![host_snapshot()];
    assert!(
        validate_linux_package_manifest(&manifest).is_err(),
        "policy none must not carry snapshots"
    );

    let mut manifest = shipped_manifest();
    manifest.snapshot_policy = "embedded".to_string();
    assert!(
        validate_linux_package_manifest(&manifest).is_err(),
        "policy embedded must carry a snapshot"
    );
}

#[test]
fn an_embedded_snapshot_must_match_the_package_arch() {
    let mut manifest = shipped_manifest();
    manifest.snapshot_policy = "embedded".to_string();
    manifest.snapshots = vec![host_snapshot()];
    validate_linux_package_manifest(&manifest)
        .expect("a matching host snapshot is the supported case");

    // A snapshot is V8 machine code; one for another arch is not loadable.
    let mut mismatched = manifest.clone();
    mismatched.snapshots[0].arch = "aarch64".to_string();
    assert!(
        validate_linux_package_manifest(&mismatched).is_err(),
        "a snapshot for another arch must be rejected"
    );

    let mut mismatched = manifest.clone();
    mismatched.snapshots[0].target_triple = "aarch64-linux-android".to_string();
    assert!(
        validate_linux_package_manifest(&mismatched).is_err(),
        "a snapshot for another triple must be rejected"
    );
}

#[test]
fn dynamic_dependencies_must_be_recorded_sorted_and_unique() {
    let mut manifest = shipped_manifest();
    manifest.dynamic_dependencies.clear();
    assert!(
        validate_linux_package_manifest(&manifest).is_err(),
        "an empty DT_NEEDED list means the manifest was never filled in"
    );

    let mut manifest = shipped_manifest();
    manifest.dynamic_dependencies = vec!["libc.so.6".to_string(), "libEGL.so.1".to_string()];
    assert!(
        validate_linux_package_manifest(&manifest).is_err(),
        "unsorted dependencies must be rejected so comparisons stay meaningful"
    );
}

#[test]
fn a_zero_length_artifact_is_not_a_shipped_binary() {
    let mut manifest = shipped_manifest();
    manifest.artifacts.insert("libmigo.so.0.1.0".to_string(), 0);
    assert!(
        validate_linux_package_manifest(&manifest).is_err(),
        "a zero-byte artifact means the build produced nothing"
    );
}

fn host_snapshot() -> PackageSnapshotIdentity {
    PackageSnapshotIdentity {
        runtime_kind: "host".to_string(),
        target_triple: "x86_64-unknown-linux-gnu".to_string(),
        arch: "x86_64".to_string(),
        normalized_parameters: vec![
            "--arch=x86_64".to_string(),
            "--cpu-policy=target-baseline".to_string(),
        ],
        bytes_hash: "0".repeat(64),
    }
}
