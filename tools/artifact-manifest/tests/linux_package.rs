//! The Linux GNU package manifest contract.
//!
//! The failures worth catching here are not malformed JSON -- serde rejects
//! that -- but manifests that parse cleanly and describe a package that cannot
//! run: a V8 built for another target, a snapshot for another architecture, a
//! loader floor quietly raised past what consumers were promised.

mod common;

use artifact_manifest::{
    LinuxPackageManifest, PackageArtifactIdentity, PackageSnapshotIdentity, sha256_file,
    validate_linux_package_manifest, verify_linux_package,
};
use common::{
    LINUX_SYSROOT_IDENTITY, android_v8_component, gles_graphics, linux_v8_component,
    migo_package_provenance, sha,
};
use std::{
    collections::BTreeMap,
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
fn stage_manifest_and_links(root: &std::path::Path, manifest: &LinuxPackageManifest) {
    let manifest_path = root.join("share/migo/linux-x86_64-manifest.json");
    fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
    fs::write(manifest_path, serde_json::to_vec_pretty(manifest).unwrap()).unwrap();
    std::os::unix::fs::symlink("libmigo.so.1", root.join("lib/libmigo.so")).unwrap();
    std::os::unix::fs::symlink(
        format!("libmigo.so.{}", manifest.version),
        root.join("lib/libmigo.so.1"),
    )
    .unwrap();
}

fn shipped_manifest() -> LinuxPackageManifest {
    let v8 = linux_v8_component();
    LinuxPackageManifest {
        schema: "migo-linux-package-manifest/v2".to_string(),
        version: "0.1.0".to_string(),
        product_profile: "full".to_string(),
        build_type: "release".to_string(),
        codegen_profile: "z".to_string(),
        target: "x86_64-unknown-linux-gnu".to_string(),
        os: "linux".to_string(),
        abi: "gnu".to_string(),
        arch: "x86_64".to_string(),
        cpu_baseline: "x86-64-v1".to_string(),
        required_cpu_features: vec!["cmov".to_string(), "sse2".to_string()],
        glibc_floor: "2.31".to_string(),
        glibcxx_floor: "3.4.28".to_string(),
        sysroot: LINUX_SYSROOT_IDENTITY.to_string(),
        dynamic_dependencies: vec![
            "libEGL.so.1".to_string(),
            "libc.so.6".to_string(),
            "libstdc++.so.6".to_string(),
        ],
        snapshot_policy: "none".to_string(),
        snapshots: Vec::new(),
        toolchain: v8.toolchain.clone(),
        v8,
        graphics: gles_graphics(),
        provenance: migo_package_provenance("scripts/build-linux-sdk.sh"),
        artifacts: BTreeMap::from([
            (
                "lib/libmigo.a".to_string(),
                PackageArtifactIdentity {
                    size_bytes: 1024,
                    sha256: sha('a'),
                },
            ),
            (
                "lib/libmigo.so.0.1.0".to_string(),
                PackageArtifactIdentity {
                    size_bytes: 2048,
                    sha256: sha('b'),
                },
            ),
        ]),
    }
}

#[test]
fn the_shipped_manifest_shape_is_valid() {
    let manifest = shipped_manifest();
    let wire = serde_json::to_vec(&manifest).expect("serialize package manifest");
    let decoded: LinuxPackageManifest =
        serde_json::from_slice(&wire).expect("deserialize package manifest");
    validate_linux_package_manifest(&decoded).expect("shipped manifest must validate");
}

#[test]
fn a_v8_built_for_another_target_cannot_be_shipped_in_this_package() {
    let mut manifest = shipped_manifest();
    manifest.v8 = android_v8_component("aarch64");
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
fn engine_and_v8_must_use_the_same_sysroot_identity() {
    let mut manifest = shipped_manifest();
    manifest.sysroot = "different-sysroot-recipe".to_string();
    manifest.toolchain.sdk = manifest.sysroot.clone();
    let error = validate_linux_package_manifest(&manifest)
        .expect_err("engine and V8 sysroot identities must not diverge");
    assert!(
        error.to_string().contains("v8.toolchain.sdk"),
        "error should name the cross-component identity mismatch, got: {error}"
    );
}

#[test]
fn package_version_is_strict_semver_and_names_the_real_soname_file() {
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
            validate_linux_package_manifest(&manifest).is_err(),
            "unsafe or non-SemVer package version must be rejected: {invalid:?}"
        );
    }

    let mut prerelease = shipped_manifest();
    prerelease.version = "1.2.3-rc-alpha.1+build.7".to_string();
    prerelease.artifacts.remove("lib/libmigo.so.0.1.0");
    prerelease.artifacts.insert(
        "lib/libmigo.so.1.2.3-rc-alpha.1+build.7".to_string(),
        PackageArtifactIdentity {
            size_bytes: 2048,
            sha256: sha('b'),
        },
    );
    validate_linux_package_manifest(&prerelease).expect("valid SemVer must be accepted");

    let mut wrong_file = shipped_manifest();
    let identity = wrong_file.artifacts.remove("lib/libmigo.so.0.1.0").unwrap();
    wrong_file
        .artifacts
        .insert("lib/libmigo.so.unrelated".to_string(), identity);
    assert!(
        validate_linux_package_manifest(&wrong_file).is_err(),
        "the real shared object must be named from the manifest version"
    );
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
    manifest
        .artifacts
        .get_mut("lib/libmigo.so.0.1.0")
        .expect("fixture shared library")
        .size_bytes = 0;
    assert!(
        validate_linux_package_manifest(&manifest).is_err(),
        "a zero-byte artifact means the build produced nothing"
    );
}

#[test]
fn legacy_size_only_artifacts_are_rejected() {
    let mut value = serde_json::to_value(shipped_manifest()).unwrap();
    value["artifacts"]["lib/libmigo.a"] = serde_json::json!(1024);
    assert!(
        serde_json::from_value::<LinuxPackageManifest>(value).is_err(),
        "a byte count without a content hash is not an artifact identity"
    );
}

#[test]
fn exact_v8_build_identity_is_mandatory_on_the_wire() {
    let mut value = serde_json::to_value(shipped_manifest()).unwrap();
    value["v8"]["runtime"]
        .as_object_mut()
        .unwrap()
        .remove("v8_revision");
    let value: LinuxPackageManifest = serde_json::from_value(value).unwrap();
    assert!(
        validate_linux_package_manifest(&value).is_err(),
        "the upstream V8 revision cannot be inferred from rusty_v8's revision"
    );
}

#[cfg(unix)]
#[test]
fn verification_binds_the_manifest_to_staged_file_bytes() {
    let mut manifest = shipped_manifest();
    let root = std::env::temp_dir().join(format!(
        "migo-linux-package-integrity-{}-{}",
        std::process::id(),
        NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed),
    ));
    for (index, (relative, identity)) in manifest.artifacts.iter_mut().enumerate() {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("artifact-{index}")).unwrap();
        identity.size_bytes = fs::metadata(&path).unwrap().len();
        identity.sha256 = sha256_file(&path).unwrap();
    }
    stage_manifest_and_links(&root, &manifest);

    verify_linux_package(&manifest, &root).expect("materialized package must verify");
    fs::write(root.join("lib/libmigo.a"), b"tampered").unwrap();
    let error = verify_linux_package(&manifest, &root).unwrap_err();
    assert!(
        error.to_string().contains("sha256") || error.to_string().contains("size_bytes"),
        "tampering error must name the broken content identity: {error}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn verification_rejects_an_undeclared_regular_file_in_the_package_tree() {
    let mut manifest = shipped_manifest();
    let root = std::env::temp_dir().join(format!(
        "migo-linux-package-closure-{}-{}",
        std::process::id(),
        NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed),
    ));
    for (index, (relative, identity)) in manifest.artifacts.iter_mut().enumerate() {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("artifact-{index}")).unwrap();
        identity.size_bytes = fs::metadata(&path).unwrap().len();
        identity.sha256 = sha256_file(&path).unwrap();
    }
    stage_manifest_and_links(&root, &manifest);
    fs::write(root.join("undeclared.txt"), b"not in the manifest").unwrap();

    let error = verify_linux_package(&manifest, &root).unwrap_err();
    assert!(error.to_string().contains("undeclared regular file"));
    fs::remove_file(root.join("undeclared.txt")).unwrap();
    fs::remove_file(root.join("lib/libmigo.so")).unwrap();
    std::os::unix::fs::symlink("libmigo.so.evil", root.join("lib/libmigo.so")).unwrap();
    let error = verify_linux_package(&manifest, &root).unwrap_err();
    assert!(error.to_string().contains("symlink target mismatch"));
    fs::remove_dir_all(root).unwrap();
}

fn host_snapshot() -> PackageSnapshotIdentity {
    PackageSnapshotIdentity {
        runtime_kind: "host".to_string(),
        product_profile: "full".to_string(),
        target_triple: "x86_64-unknown-linux-gnu".to_string(),
        arch: "x86_64".to_string(),
        schema: "3".to_string(),
        generator: "migo-snapshot-gen/0.1.0".to_string(),
        generation_cpu_policy: "target-baseline".to_string(),
        normalized_parameters: vec![
            "--arch=x86_64".to_string(),
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
        v8_archive_hash: sha('d'),
        bytes_size: 4096,
        bytes_hash: sha('5'),
        js_sources_hash: sha('6'),
        deno_core_version: "0.385.0".to_string(),
    }
}
