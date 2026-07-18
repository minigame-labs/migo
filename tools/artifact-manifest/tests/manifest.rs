mod common;

use artifact_manifest::{canonical_identity_json, seal_slice_manifest, validate_slice_manifest};
use common::{android_manifest, sha};

#[test]
fn accepts_android_api26_aarch64_and_x86_64_slices() {
    validate_slice_manifest(&android_manifest("aarch64")).expect("aarch64 is valid");
    validate_slice_manifest(&android_manifest("x86_64")).expect("x86_64 is valid");
}

#[test]
fn canonical_identity_is_stable_for_runtime_floor_key_order() {
    let first = android_manifest("aarch64");
    let mut second = first.clone();
    second.target.runtime_floor = [
        ("z_future".to_string(), "off".to_string()),
        ("android_api".to_string(), "26".to_string()),
    ]
    .into_iter()
    .collect();
    let mut first_with_future = first;
    first_with_future
        .target
        .runtime_floor
        .insert("z_future".to_string(), "off".to_string());

    assert_eq!(
        canonical_identity_json(&first_with_future).unwrap(),
        canonical_identity_json(&second).unwrap()
    );
}

#[test]
fn rejects_tampered_identity() {
    let mut manifest = android_manifest("aarch64");
    manifest.hashes.runtime_binary = sha('a');
    let error = validate_slice_manifest(&manifest).unwrap_err();
    assert!(error.to_string().contains("artifact_id"));
}

#[test]
fn rejects_android_floor_other_than_26() {
    let mut manifest = android_manifest("aarch64");
    manifest
        .target
        .runtime_floor
        .insert("android_api".to_string(), "27".to_string());
    seal_slice_manifest(&mut manifest).unwrap();
    let error = validate_slice_manifest(&manifest).unwrap_err();
    assert!(error.to_string().contains("android_api"));
}

#[test]
fn rejects_wrong_cpu_baseline_for_architecture() {
    let mut manifest = android_manifest("x86_64");
    manifest.target.cpu_baseline = "x86-64-v3".to_string();
    seal_slice_manifest(&mut manifest).unwrap();
    let error = validate_slice_manifest(&manifest).unwrap_err();
    assert!(error.to_string().contains("cpu_baseline"));
}

#[test]
fn rejects_placeholder_v8_revision() {
    let mut manifest = android_manifest("aarch64");
    manifest.runtime.v8_revision = Some("unknown".to_string());
    seal_slice_manifest(&mut manifest).unwrap();
    let error = validate_slice_manifest(&manifest).unwrap_err();
    assert!(error.to_string().contains("v8_revision"));
}

#[test]
fn rejects_unsorted_gn_arguments() {
    let mut manifest = android_manifest("aarch64");
    manifest.runtime.normalized_gn_args.reverse();
    seal_slice_manifest(&mut manifest).unwrap();
    let error = validate_slice_manifest(&manifest).unwrap_err();
    assert!(error.to_string().contains("normalized_gn_args"));
}

#[test]
fn rejects_duplicate_gn_argument_keys() {
    let mut manifest = android_manifest("aarch64");
    manifest
        .runtime
        .normalized_gn_args
        .push("is_official_build=false".to_string());
    manifest.runtime.normalized_gn_args.sort();
    seal_slice_manifest(&mut manifest).unwrap();
    let error = validate_slice_manifest(&manifest).unwrap_err();
    assert!(error.to_string().contains("duplicate GN argument key"));
}

#[test]
fn rejects_duplicate_snapshot_runtime_kind() {
    let mut manifest = android_manifest("aarch64");
    manifest.snapshots.push(manifest.snapshots[0].clone());
    seal_slice_manifest(&mut manifest).unwrap();
    let error = validate_slice_manifest(&manifest).unwrap_err();
    assert!(error.to_string().contains("snapshot runtime_kind"));
}

#[test]
fn rejects_unknown_wire_fields_instead_of_silently_ignoring_them() {
    let mut value = serde_json::to_value(android_manifest("aarch64")).unwrap();
    value.as_object_mut().unwrap().insert(
        "future_identity".to_string(),
        serde_json::json!("unreviewed"),
    );

    let error = serde_json::from_value::<artifact_manifest::SliceManifest>(value).unwrap_err();
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn rejects_snapshot_parameters_for_a_different_architecture() {
    let mut manifest = android_manifest("aarch64");
    manifest.snapshots[0].normalized_parameters[0] = "--arch=x86_64".to_string();
    seal_slice_manifest(&mut manifest).unwrap();

    let error = validate_slice_manifest(&manifest).unwrap_err();
    assert!(error.to_string().contains("snapshot") && error.to_string().contains("arch"));
}

#[test]
fn rejects_snapshot_target_identity_for_a_different_architecture() {
    let mut manifest = android_manifest("aarch64");
    manifest.snapshots[0].arch = "x86_64".to_string();
    manifest.snapshots[0].target_triple = "x86_64-linux-android".to_string();
    seal_slice_manifest(&mut manifest).unwrap();

    let error = validate_slice_manifest(&manifest).unwrap_err();
    assert!(error.to_string().contains("snapshot.target_triple"));
}
