use std::collections::BTreeMap;

use artifact_manifest::{
    ArtifactHashes, GraphicsIdentity, PatchIdentity, ProvenanceIdentity, RuntimeIdentity,
    SliceManifest, SnapshotIdentity, TargetIdentity, ToolchainIdentity, V8ComponentHashes,
    V8ComponentManifest, seal_slice_manifest, seal_v8_component_manifest,
};

pub fn sha(byte: char) -> String {
    std::iter::repeat_n(byte, 64).collect()
}

pub const LINUX_SYSROOT_IDENTITY: &str = "Debian bullseye amd64 sysroot; sysroots.json sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[allow(dead_code)]
pub fn gles_graphics() -> GraphicsIdentity {
    GraphicsIdentity {
        backend_family: "gles-native".to_string(),
        required_api: "OpenGL ES 3.0".to_string(),
    }
}

#[allow(dead_code)]
pub fn migo_package_provenance(recipe: &str) -> ProvenanceIdentity {
    ProvenanceIdentity {
        source_revision: "0123456789abcdef0123456789abcdef01234567".to_string(),
        build_recipe: recipe.to_string(),
        build_recipe_sha256: sha('9'),
        licenses: vec![
            "Apache-2.0".to_string(),
            "BSD-3-Clause".to_string(),
            "BSL-1.1".to_string(),
            "MIT".to_string(),
        ],
    }
}

pub fn android_manifest(arch: &str) -> SliceManifest {
    let (triple, cpu_baseline, required_cpu_features) = match arch {
        "aarch64" => ("aarch64-linux-android", "armv8-a", vec!["neon".to_string()]),
        "x86_64" => (
            "x86_64-linux-android",
            "x86-64-v1",
            vec!["cmov".to_string(), "sse2".to_string()],
        ),
        other => panic!("unexpected test architecture: {other}"),
    };

    let mut runtime_floor = BTreeMap::new();
    runtime_floor.insert("android_api".to_string(), "26".to_string());

    let mut manifest = SliceManifest {
        schema: "migo-artifact-manifest/v1".to_string(),
        artifact_id: String::new(),
        product_profile: "full".to_string(),
        build_type: "release".to_string(),
        codegen_profile: "z".to_string(),
        target: TargetIdentity {
            triple: triple.to_string(),
            os: "android".to_string(),
            arch: arch.to_string(),
            abi: "android".to_string(),
            cpu_baseline: cpu_baseline.to_string(),
            required_cpu_features,
            runtime_floor,
        },
        toolchain: ToolchainIdentity {
            rustc: "rustc 1.95.0 (deadbeef 2026-07-01)".to_string(),
            compiler: "Android clang version 12.0.8".to_string(),
            sdk: "Android NDK 23.2.8568313; API 26 sysroot".to_string(),
            linker: "LLD 12.0.8".to_string(),
        },
        runtime: RuntimeIdentity {
            backend: "v8".to_string(),
            rusty_v8_version: Some("145.0.0".to_string()),
            rusty_v8_revision: Some("e6a88b35dd3d7f2849a0df33a71d338701c55316".to_string()),
            v8_revision: Some("8defb67673c5483ae56258a2de01b07e947dc921".to_string()),
            normalized_gn_args: vec![
                "android_ndk_api_level=26".to_string(),
                "is_official_build=true".to_string(),
                "use_thin_lto=false".to_string(),
            ],
            patches: vec![PatchIdentity {
                id: "0001-unset-bindgen-extra-clang-args".to_string(),
                sha256: sha('1'),
            }],
        },
        snapshots: vec![SnapshotIdentity {
            runtime_kind: "host".to_string(),
            product_profile: "full".to_string(),
            target_triple: triple.to_string(),
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
            external_references_hash: sha('2'),
            bootstrap_inputs_hash: sha('3'),
            bytes_hash: sha('4'),
        }],
        graphics: GraphicsIdentity {
            backend_family: "gles-native".to_string(),
            required_api: "OpenGL ES 3.0".to_string(),
        },
        hashes: ArtifactHashes {
            runtime_binary: sha('5'),
            v8_archive: Some(sha('6')),
            rust_binding: Some(sha('7')),
            cxx_runtime: Some(sha('8')),
        },
        provenance: ProvenanceIdentity {
            source_revision: "0123456789abcdef0123456789abcdef01234567".to_string(),
            build_recipe: "scripts/build-aar.sh".to_string(),
            build_recipe_sha256: sha('9'),
            licenses: vec![
                "Apache-2.0".to_string(),
                "BSD-3-Clause".to_string(),
                "BSL-1.1".to_string(),
                "MIT".to_string(),
            ],
        },
    };
    seal_slice_manifest(&mut manifest).expect("seal valid test manifest");
    manifest
}

#[allow(dead_code)]
pub fn android_v8_component(arch: &str) -> V8ComponentManifest {
    let slice = android_manifest(arch);
    let mut component = V8ComponentManifest {
        schema: "migo-v8-component-manifest/v1".to_string(),
        component_id: String::new(),
        target: slice.target,
        toolchain: slice.toolchain,
        runtime: slice.runtime,
        hashes: V8ComponentHashes {
            archive: sha('6'),
            rust_binding: sha('7'),
        },
        provenance: ProvenanceIdentity {
            source_revision: "e6a88b35dd3d7f2849a0df33a71d338701c55316".to_string(),
            build_recipe: "scripts/build-v8-android.sh".to_string(),
            build_recipe_sha256: sha('a'),
            licenses: vec!["BSD-3-Clause".to_string(), "MIT".to_string()],
        },
    };
    seal_v8_component_manifest(&mut component).expect("seal valid V8 component");
    component
}

#[allow(dead_code)]
pub fn linux_v8_component() -> V8ComponentManifest {
    let mut runtime_floor = BTreeMap::new();
    runtime_floor.insert("glibc".to_string(), "2.31".to_string());
    runtime_floor.insert("glibcxx".to_string(), "3.4.28".to_string());

    let rusty_v8_revision = "0b8cfc5ae9d2507031076df2acdf61b0742a4c4e";
    let mut component = V8ComponentManifest {
        schema: "migo-v8-component-manifest/v1".to_string(),
        component_id: String::new(),
        target: TargetIdentity {
            triple: "x86_64-unknown-linux-gnu".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            abi: "gnu".to_string(),
            cpu_baseline: "x86-64-v1".to_string(),
            required_cpu_features: vec!["cmov".to_string(), "sse2".to_string()],
            runtime_floor,
        },
        toolchain: ToolchainIdentity {
            rustc: "rustc 1.95.0 fixture".to_string(),
            compiler: "clang 19 fixture".to_string(),
            sdk: LINUX_SYSROOT_IDENTITY.to_string(),
            linker: "LLD 19 fixture".to_string(),
        },
        runtime: RuntimeIdentity {
            backend: "v8".to_string(),
            rusty_v8_version: Some("145.0.0".to_string()),
            rusty_v8_revision: Some(rusty_v8_revision.to_string()),
            v8_revision: Some("8defb67673c5483ae56258a2de01b07e947dc921".to_string()),
            normalized_gn_args: vec![
                "is_official_build=true".to_string(),
                "use_sysroot=true".to_string(),
                "v8_monolithic_for_shared_library=true".to_string(),
            ],
            patches: vec![PatchIdentity {
                id: "migo-build-rs-prebuilt-binding".to_string(),
                sha256: sha('c'),
            }],
        },
        hashes: V8ComponentHashes {
            archive: sha('d'),
            rust_binding: sha('e'),
        },
        provenance: ProvenanceIdentity {
            source_revision: rusty_v8_revision.to_string(),
            build_recipe: "scripts/build-v8-linux.sh".to_string(),
            build_recipe_sha256: sha('f'),
            licenses: vec!["BSD-3-Clause".to_string(), "MIT".to_string()],
        },
    };
    seal_v8_component_manifest(&mut component).expect("seal valid Linux V8 component");
    component
}
