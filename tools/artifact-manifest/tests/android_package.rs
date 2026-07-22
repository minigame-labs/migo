//! The Android C ABI package manifest contract.
//!
//! Android's C ABI ships differently from Linux's: it is cross-compiled, the
//! primary artifact is a static library a host links into its own `.so`, and it
//! **embeds** a V8 startup snapshot (Linux embeds none). The failures worth
//! catching are the same class as the Linux side -- a manifest that parses
//! cleanly but describes something that cannot run: a V8 or snapshot built for
//! another target, an API floor below the project minimum, an embedded-snapshot
//! claim with no snapshot behind it.

mod common;

use artifact_manifest::{
    AndroidPackageManifest, PackageArtifactIdentity, PackageSnapshotIdentity, sha256_file,
    validate_android_package_manifest, verify_android_package,
};
use common::{android_v8_component, gles_graphics, migo_package_provenance, sha};
use std::{
    collections::BTreeMap,
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

fn stage_manifest(root: &std::path::Path, manifest: &AndroidPackageManifest) {
    let path = root.join("share/migo/android-arm64-v8a-manifest.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(manifest).unwrap()).unwrap();
}

fn shipped_manifest() -> AndroidPackageManifest {
    let v8 = android_v8_component("aarch64");
    AndroidPackageManifest {
        schema: "migo-android-package-manifest/v2".to_string(),
        version: "0.1.0".to_string(),
        product_profile: "full".to_string(),
        build_type: "release".to_string(),
        codegen_profile: "z".to_string(),
        os: "android".to_string(),
        abi: "android".to_string(),
        arch: "aarch64".to_string(),
        android_abi: "arm64-v8a".to_string(),
        target_triple: "aarch64-linux-android".to_string(),
        cpu_baseline: "armv8-a".to_string(),
        required_cpu_features: vec!["neon".to_string()],
        min_android_api: "26".to_string(),
        link_libraries: vec![
            "-lEGL".to_string(),
            "-lGLESv2".to_string(),
            "-lOpenSLES".to_string(),
            "-landroid".to_string(),
            "-llog".to_string(),
        ],
        snapshot_policy: "embedded".to_string(),
        snapshots: vec![host_snapshot("aarch64")],
        toolchain: v8.toolchain.clone(),
        v8,
        graphics: gles_graphics(),
        provenance: migo_package_provenance("scripts/build-android-sdk.sh"),
        artifacts: BTreeMap::from([(
            "lib/libmigo_capi.a".to_string(),
            PackageArtifactIdentity {
                size_bytes: 120_000_000,
                sha256: sha('a'),
            },
        )]),
    }
}

#[test]
fn the_shipped_manifest_shape_is_valid() {
    let manifest = shipped_manifest();
    let wire = serde_json::to_vec(&manifest).expect("serialize package manifest");
    let decoded: AndroidPackageManifest =
        serde_json::from_slice(&wire).expect("deserialize package manifest");
    validate_android_package_manifest(&decoded).expect("shipped manifest must validate");
}

#[test]
fn a_v8_built_for_another_target_cannot_be_shipped_in_this_package() {
    let mut manifest = shipped_manifest();
    manifest.v8 = android_v8_component("x86_64");
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
fn package_version_is_strict_semver() {
    for invalid in [
        "1",
        "1.0",
        "01.0.0",
        "1.0.0-",
        "1.0.0/../../escape",
        "1.0.0\nset(MIGO_FOUND TRUE)",
    ] {
        let mut manifest = shipped_manifest();
        manifest.version = invalid.to_string();
        assert!(
            validate_android_package_manifest(&manifest).is_err(),
            "unsafe or non-SemVer package version must be rejected: {invalid:?}"
        );
    }

    let mut prerelease = shipped_manifest();
    prerelease.version = "1.2.3-rc-alpha.1+build.7".to_string();
    validate_android_package_manifest(&prerelease).expect("valid SemVer must be accepted");
}

#[test]
fn android_always_embeds_a_snapshot_and_it_must_match_the_arch() {
    // Unlike Linux, an Android package with no snapshot is a defect, not a
    // policy: runtime-v8 embeds one for every Android ABI.
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
fn package_level_graphics_and_license_contracts_cannot_drift() {
    let mut manifest = shipped_manifest();
    manifest.graphics.backend_family = "angle".to_string();
    let error = validate_android_package_manifest(&manifest)
        .expect_err("the Android package must describe the native GLES backend it ships");
    assert!(
        error.to_string().contains("graphics.backend_family"),
        "error should name the graphics contract, got: {error}"
    );

    let mut manifest = shipped_manifest();
    manifest
        .provenance
        .licenses
        .retain(|license| license != "BSL-1.1");
    let error = validate_android_package_manifest(&manifest)
        .expect_err("the package must record Migo's current repository license");
    assert!(
        error.to_string().contains("BSL-1.1"),
        "error should name the missing current license, got: {error}"
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
    manifest.v8 = android_v8_component("x86_64");
    manifest.snapshots = vec![host_snapshot("x86_64")];
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
    manifest
        .artifacts
        .get_mut("lib/libmigo_capi.a")
        .expect("fixture static library")
        .size_bytes = 0;
    assert!(
        validate_android_package_manifest(&manifest).is_err(),
        "a zero-byte static library means the cross-compile produced nothing"
    );
}

#[test]
fn legacy_size_only_artifacts_are_rejected() {
    let mut value = serde_json::to_value(shipped_manifest()).unwrap();
    value["artifacts"]["lib/libmigo_capi.a"] = serde_json::json!(120000000);
    assert!(
        serde_json::from_value::<AndroidPackageManifest>(value).is_err(),
        "a byte count without a content hash is not an artifact identity"
    );
}

#[test]
fn exact_v8_and_snapshot_build_identity_are_mandatory_on_the_wire() {
    let mut missing_v8_revision = serde_json::to_value(shipped_manifest()).unwrap();
    missing_v8_revision["v8"]["runtime"]
        .as_object_mut()
        .unwrap()
        .remove("v8_revision");
    let missing_v8_revision: AndroidPackageManifest =
        serde_json::from_value(missing_v8_revision).unwrap();
    assert!(
        validate_android_package_manifest(&missing_v8_revision).is_err(),
        "the upstream V8 revision is mandatory"
    );

    let mut missing_snapshot_inputs = serde_json::to_value(shipped_manifest()).unwrap();
    missing_snapshot_inputs["snapshots"][0]
        .as_object_mut()
        .unwrap()
        .remove("bootstrap_inputs_hash");
    assert!(
        serde_json::from_value::<AndroidPackageManifest>(missing_snapshot_inputs).is_err(),
        "a snapshot hash without its generation-input identity is incomplete"
    );
}

#[test]
fn verification_binds_the_manifest_to_the_staged_static_library() {
    let mut manifest = shipped_manifest();
    let root = std::env::temp_dir().join(format!(
        "migo-android-package-integrity-{}-{}",
        std::process::id(),
        NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed),
    ));
    let path = root.join("lib/libmigo_capi.a");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"android-static-library").unwrap();
    let identity = manifest
        .artifacts
        .get_mut("lib/libmigo_capi.a")
        .expect("fixture static library");
    identity.size_bytes = fs::metadata(&path).unwrap().len();
    identity.sha256 = sha256_file(&path).unwrap();
    stage_manifest(&root, &manifest);

    verify_android_package(&manifest, &root).expect("materialized package must verify");
    fs::write(&path, b"tampered-static-library").unwrap();
    let error = verify_android_package(&manifest, &root).unwrap_err();
    assert!(
        error.to_string().contains("sha256") || error.to_string().contains("size_bytes"),
        "tampering error must name the broken content identity: {error}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn verification_rejects_an_undeclared_regular_file_in_the_package_tree() {
    let mut manifest = shipped_manifest();
    let root = std::env::temp_dir().join(format!(
        "migo-android-package-closure-{}-{}",
        std::process::id(),
        NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed),
    ));
    let path = root.join("lib/libmigo_capi.a");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"android-static-library").unwrap();
    let identity = manifest.artifacts.get_mut("lib/libmigo_capi.a").unwrap();
    identity.size_bytes = fs::metadata(&path).unwrap().len();
    identity.sha256 = sha256_file(&path).unwrap();
    stage_manifest(&root, &manifest);
    fs::write(root.join("undeclared.txt"), b"not in the manifest").unwrap();

    let error = verify_android_package(&manifest, &root).unwrap_err();
    assert!(error.to_string().contains("undeclared regular file"));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn verification_rejects_symlinks_and_a_different_packaged_manifest() {
    let mut manifest = shipped_manifest();
    let root = std::env::temp_dir().join(format!(
        "migo-android-package-topology-{}-{}",
        std::process::id(),
        NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed),
    ));
    let path = root.join("lib/libmigo_capi.a");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"android-static-library").unwrap();
    let identity = manifest.artifacts.get_mut("lib/libmigo_capi.a").unwrap();
    identity.size_bytes = fs::metadata(&path).unwrap().len();
    identity.sha256 = sha256_file(&path).unwrap();
    stage_manifest(&root, &manifest);

    std::os::unix::fs::symlink("libmigo_capi.a", root.join("lib/alias.a")).unwrap();
    let error = verify_android_package(&manifest, &root).unwrap_err();
    assert!(error.to_string().contains("undeclared symlink"));
    fs::remove_file(root.join("lib/alias.a")).unwrap();

    let mut different = manifest.clone();
    different.version = "0.1.1".to_string();
    let error = verify_android_package(&different, &root).unwrap_err();
    assert!(error.to_string().contains("does not match"));
    fs::remove_dir_all(root).unwrap();
}

fn host_snapshot(arch: &str) -> PackageSnapshotIdentity {
    let target_triple = match arch {
        "aarch64" => "aarch64-linux-android",
        "x86_64" => "x86_64-linux-android",
        other => panic!("unexpected Android test arch: {other}"),
    };
    PackageSnapshotIdentity {
        runtime_kind: "host".to_string(),
        product_profile: "full".to_string(),
        target_triple: target_triple.to_string(),
        arch: arch.to_string(),
        schema: "3".to_string(),
        generator: "migo-snapshot-gen/0.1.0".to_string(),
        generation_cpu_policy: "target-baseline".to_string(),
        normalized_parameters: vec![
            format!("--arch={arch}"),
            "--cpu-policy=target-baseline".to_string(),
            "--product-profile=full".to_string(),
            "--runtime-kind=host".to_string(),
            "--warmup=none".to_string(),
        ],
        external_references_hash: sha('1'),
        bootstrap_inputs_hash: sha('2'),
        features: vec!["profile-full".to_string()],
        features_hash: sha('3'),
        rust_sources_hash: sha('4'),
        v8_archive_hash: sha('6'),
        bytes_size: 4096,
        bytes_hash: sha('5'),
        js_sources_hash: sha('7'),
        deno_core_version: "0.385.0".to_string(),
    }
}
