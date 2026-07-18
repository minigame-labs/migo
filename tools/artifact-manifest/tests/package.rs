mod common;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use artifact_manifest::{
    SliceManifestSource, build_package_index, build_release_attestation, sha256_file,
    verify_package_index, verify_release_attestation,
};
use common::android_manifest;

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "migo-artifact-manifest-{label}-{}-{sequence}",
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

fn write_manifest(path: &Path, arch: &str) {
    let bytes = serde_json::to_vec_pretty(&android_manifest(arch)).expect("serialize manifest");
    fs::write(path, bytes).expect("write manifest");
}

#[test]
fn package_index_hashes_manifests_in_stable_target_order() {
    let directory = TestDir::new("index-order");
    let arm_path = directory.path().join("arm64-v8a.json");
    let x64_path = directory.path().join("x86_64.json");
    write_manifest(&arm_path, "aarch64");
    write_manifest(&x64_path, "x86_64");

    let sources = [
        SliceManifestSource {
            package_path: "META-INF/migo/slices/x86_64.json".to_string(),
            file_path: x64_path.clone(),
        },
        SliceManifestSource {
            package_path: "META-INF/migo/slices/arm64-v8a.json".to_string(),
            file_path: arm_path.clone(),
        },
    ];
    let index = build_package_index("full", &sources).expect("build package index");

    assert_eq!(index.slices.len(), 2);
    assert_eq!(index.build_type, "release");
    assert_eq!(index.codegen_profile, "z");
    assert_eq!(index.slices[0].arch, "aarch64");
    assert_eq!(index.slices[1].arch, "x86_64");
    assert_eq!(
        index.slices[0].manifest_sha256,
        sha256_file(&arm_path).unwrap()
    );
    assert_eq!(
        index.slices[1].manifest_sha256,
        sha256_file(&x64_path).unwrap()
    );

    let package_root = directory.path().join("package");
    fs::create_dir_all(package_root.join("META-INF/migo/slices")).unwrap();
    fs::copy(
        &arm_path,
        package_root.join("META-INF/migo/slices/arm64-v8a.json"),
    )
    .unwrap();
    fs::copy(
        &x64_path,
        package_root.join("META-INF/migo/slices/x86_64.json"),
    )
    .unwrap();
    verify_package_index(&index, &package_root).expect("verify package index");
}

#[test]
fn package_index_detects_manifest_tampering() {
    let directory = TestDir::new("index-tamper");
    let manifest_path = directory.path().join("slice.json");
    write_manifest(&manifest_path, "aarch64");
    let index = build_package_index(
        "full",
        &[SliceManifestSource {
            package_path: "META-INF/migo/slices/arm64-v8a.json".to_string(),
            file_path: manifest_path.clone(),
        }],
    )
    .unwrap();

    let package_path = directory.path().join("META-INF/migo/slices/arm64-v8a.json");
    fs::create_dir_all(package_path.parent().unwrap()).unwrap();
    fs::copy(&manifest_path, &package_path).unwrap();
    let mut tampered = android_manifest("aarch64");
    tampered.toolchain.sdk = "Android NDK tampered".to_string();
    fs::write(&package_path, serde_json::to_vec_pretty(&tampered).unwrap()).unwrap();

    let error = verify_package_index(&index, directory.path()).unwrap_err();
    assert!(error.to_string().contains("manifest_sha256"));
}

#[test]
fn package_index_rejects_unsafe_embedded_paths() {
    let directory = TestDir::new("index-path");
    let manifest_path = directory.path().join("slice.json");
    write_manifest(&manifest_path, "aarch64");

    let error = build_package_index(
        "full",
        &[SliceManifestSource {
            package_path: "../slice.json".to_string(),
            file_path: manifest_path,
        }],
    )
    .unwrap_err();
    assert!(error.to_string().contains("relative package path"));
}

#[test]
fn release_attestation_hashes_the_final_package_only_outside_slice_manifests() {
    let directory = TestDir::new("attestation");
    let package_path = directory.path().join("migo-full.aar");
    let index_path = directory.path().join("package-index.json");
    fs::write(&package_path, b"aar-v1").unwrap();
    fs::write(&index_path, b"{\"schema\":\"fixture\"}").unwrap();

    let attestation =
        build_release_attestation(&package_path, &index_path).expect("build attestation");
    assert_eq!(attestation.package_file, "migo-full.aar");
    assert_eq!(attestation.package_size_bytes, 6);
    assert_eq!(
        attestation.package_sha256,
        sha256_file(&package_path).unwrap()
    );
    assert_eq!(
        attestation.package_index_sha256,
        sha256_file(&index_path).unwrap()
    );

    let embedded_slice = serde_json::to_string(&android_manifest("aarch64")).unwrap();
    assert!(!embedded_slice.contains(&attestation.package_sha256));
    verify_release_attestation(&attestation, &package_path, &index_path)
        .expect("verify attestation");
}

#[test]
fn release_attestation_detects_final_package_tampering() {
    let directory = TestDir::new("attestation-tamper");
    let package_path = directory.path().join("migo-full.aar");
    let index_path = directory.path().join("package-index.json");
    fs::write(&package_path, b"aar-v1").unwrap();
    fs::write(&index_path, b"index-v1").unwrap();
    let attestation = build_release_attestation(&package_path, &index_path).unwrap();

    fs::write(&package_path, b"aar-v2").unwrap();
    let error = verify_release_attestation(&attestation, &package_path, &index_path).unwrap_err();
    assert!(error.to_string().contains("package_sha256"));
}

#[test]
fn package_index_rejects_mixed_build_types() {
    let directory = TestDir::new("index-build-type");
    let arm_path = directory.path().join("arm64-v8a.json");
    let x64_path = directory.path().join("x86_64.json");
    write_manifest(&arm_path, "aarch64");
    let mut x64 = android_manifest("x86_64");
    x64.build_type = "debug".to_string();
    artifact_manifest::seal_slice_manifest(&mut x64).unwrap();
    fs::write(&x64_path, serde_json::to_vec_pretty(&x64).unwrap()).unwrap();

    let error = build_package_index(
        "full",
        &[
            SliceManifestSource {
                package_path: "slices/arm64-v8a.json".to_string(),
                file_path: arm_path,
            },
            SliceManifestSource {
                package_path: "slices/x86_64.json".to_string(),
                file_path: x64_path,
            },
        ],
    )
    .unwrap_err();
    assert!(error.to_string().contains("build_type"));
}

#[test]
fn package_index_rejects_mixed_codegen_profiles() {
    let directory = TestDir::new("index-codegen");
    let arm_path = directory.path().join("arm64-v8a.json");
    let x64_path = directory.path().join("x86_64.json");
    write_manifest(&arm_path, "aarch64");
    let mut x64 = android_manifest("x86_64");
    x64.codegen_profile = "3".to_string();
    artifact_manifest::seal_slice_manifest(&mut x64).unwrap();
    fs::write(&x64_path, serde_json::to_vec_pretty(&x64).unwrap()).unwrap();

    let error = build_package_index(
        "full",
        &[
            SliceManifestSource {
                package_path: "slices/arm64-v8a.json".to_string(),
                file_path: arm_path,
            },
            SliceManifestSource {
                package_path: "slices/x86_64.json".to_string(),
                file_path: x64_path,
            },
        ],
    )
    .unwrap_err();
    assert!(error.to_string().contains("codegen_profile"));
}
