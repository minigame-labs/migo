//! The Android C ABI package manifest contract.
//!
//! Android's C ABI ships differently from Linux's: it is cross-compiled, the
//! primary artifact is a static library a host links into its own `.so`, and it
//! **embeds** a V8 startup snapshot (Linux embeds none). The failures worth
//! catching are the same class as the Linux side -- a manifest that parses
//! cleanly but describes something that cannot run: a V8 or snapshot built for
//! another target, an API floor below the project minimum, an embedded-snapshot
//! claim with no snapshot behind it.

use artifact_manifest::{
    AndroidPackageManifest, PackageSnapshotIdentity, validate_android_package_manifest,
};

fn shipped_manifest() -> AndroidPackageManifest {
    serde_json::from_str(SHIPPED).expect("the shipped manifest shape must parse")
}

/// A copy of what scripts/gen-android-package-metadata.py emits for arm64-v8a.
const SHIPPED: &str = r#"{
  "schema": "migo-android-package-v1",
  "version": "0.1.0",
  "os": "android",
  "abi": "android",
  "arch": "aarch64",
  "android_abi": "arm64-v8a",
  "target_triple": "aarch64-linux-android",
  "cpu_baseline": "armv8-a",
  "required_cpu_features": ["neon"],
  "min_android_api": "26",
  "link_libraries": ["-lEGL", "-lGLESv2", "-lOpenSLES", "-landroid", "-llog"],
  "snapshot_policy": "embedded",
  "snapshots": [
    {
      "runtime_kind": "host",
      "target_triple": "aarch64-linux-android",
      "arch": "aarch64",
      "normalized_parameters": ["--arch=aarch64", "--product-profile=full"],
      "bytes_hash": "0000000000000000000000000000000000000000000000000000000000000000"
    }
  ],
  "v8": {
    "schema": "migo-v8-component-v1",
    "target": "aarch64-linux-android",
    "rusty_v8_revision": "e6a88b35dd3d7f2849a0df33a71d338701c55316"
  },
  "artifacts": { "libmigo_capi.a": 120000000 }
}"#;

#[test]
fn the_shipped_manifest_shape_is_valid() {
    validate_android_package_manifest(&shipped_manifest()).expect("shipped manifest must validate");
}

#[test]
fn a_v8_built_for_another_target_cannot_be_shipped_in_this_package() {
    let mut manifest = shipped_manifest();
    manifest.v8.target = "x86_64-linux-android".to_string();
    let error = validate_android_package_manifest(&manifest)
        .expect_err("an x86_64 V8 in an arm64 package must be rejected");
    assert!(
        error.to_string().contains("does not match package target"),
        "error should name the mismatch, got: {error}"
    );
}

#[test]
fn the_api_floor_is_pinned_to_the_project_minimum() {
    for api in ["21", "24", "27"] {
        let mut manifest = shipped_manifest();
        manifest.min_android_api = api.to_string();
        assert!(
            validate_android_package_manifest(&manifest).is_err(),
            "min_android_api {api} is not the project floor of 26"
        );
    }
}

#[test]
fn android_always_embeds_a_snapshot_and_it_must_match_the_arch() {
    // Unlike Linux, an Android package with no snapshot is a defect, not a
    // policy: js-runtime embeds one for every Android ABI.
    let mut manifest = shipped_manifest();
    manifest.snapshots.clear();
    assert!(
        validate_android_package_manifest(&manifest).is_err(),
        "embedded policy with no snapshot must be rejected"
    );

    // A snapshot is V8 machine code; one for another arch is not loadable.
    let mut manifest = shipped_manifest();
    manifest.snapshots[0].arch = "x86_64".to_string();
    assert!(
        validate_android_package_manifest(&manifest).is_err(),
        "a snapshot for another arch must be rejected"
    );

    let mut manifest = shipped_manifest();
    manifest.snapshots[0].target_triple = "x86_64-linux-android".to_string();
    assert!(
        validate_android_package_manifest(&manifest).is_err(),
        "a snapshot for another triple must be rejected"
    );
}

#[test]
fn a_none_snapshot_policy_is_not_valid_for_android() {
    // The mirror of the Linux rule: policy and content must agree, and on
    // Android the only correct policy is embedded.
    let mut manifest = shipped_manifest();
    manifest.snapshot_policy = "none".to_string();
    manifest.snapshots.clear();
    assert!(
        validate_android_package_manifest(&manifest).is_err(),
        "an Android package cannot claim to ship without a snapshot"
    );
}

#[test]
fn the_x86_64_abi_pins_its_own_triple_and_features() {
    let mut manifest = shipped_manifest();
    manifest.arch = "x86_64".to_string();
    manifest.android_abi = "x86_64".to_string();
    manifest.target_triple = "x86_64-linux-android".to_string();
    manifest.cpu_baseline = "x86-64-v1".to_string();
    manifest.required_cpu_features = vec!["cmov".to_string(), "sse2".to_string()];
    manifest.v8.target = "x86_64-linux-android".to_string();
    manifest.snapshots[0].arch = "x86_64".to_string();
    manifest.snapshots[0].target_triple = "x86_64-linux-android".to_string();
    manifest.snapshots[0].normalized_parameters = vec![
        "--arch=x86_64".to_string(),
        "--product-profile=full".to_string(),
    ];
    validate_android_package_manifest(&manifest).expect("a coherent x86_64 package is valid");

    // arch and triple must agree.
    let mut mixed = manifest.clone();
    mixed.target_triple = "aarch64-linux-android".to_string();
    assert!(
        validate_android_package_manifest(&mixed).is_err(),
        "x86_64 arch with an aarch64 triple must be rejected"
    );
}

#[test]
fn link_libraries_must_be_recorded_sorted_and_unique() {
    let mut manifest = shipped_manifest();
    manifest.link_libraries.clear();
    assert!(
        validate_android_package_manifest(&manifest).is_err(),
        "a static-lib package must record the link libraries its consumer needs"
    );

    let mut manifest = shipped_manifest();
    manifest.link_libraries = vec!["-llog".to_string(), "-landroid".to_string()];
    assert!(
        validate_android_package_manifest(&manifest).is_err(),
        "unsorted link libraries must be rejected so comparisons stay meaningful"
    );
}

#[test]
fn a_zero_length_artifact_is_not_a_shipped_static_library() {
    let mut manifest = shipped_manifest();
    manifest.artifacts.insert("libmigo_capi.a".to_string(), 0);
    assert!(
        validate_android_package_manifest(&manifest).is_err(),
        "a zero-byte static library means the cross-compile produced nothing"
    );
}

fn _host_snapshot() -> PackageSnapshotIdentity {
    PackageSnapshotIdentity {
        runtime_kind: "host".to_string(),
        target_triple: "aarch64-linux-android".to_string(),
        arch: "aarch64".to_string(),
        normalized_parameters: vec!["--arch=aarch64".to_string()],
        bytes_hash: "0".repeat(64),
    }
}
