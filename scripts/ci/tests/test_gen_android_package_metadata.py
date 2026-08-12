import hashlib
import importlib.util
import json
import pathlib
import tempfile
import unittest


_MODULE_PATH = (
    pathlib.Path(__file__).resolve().parents[1].parent
    / "gen-android-package-metadata.py"
)
_spec = importlib.util.spec_from_file_location("gen_android_package_metadata", _MODULE_PATH)
gen = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gen)

BUILD_METADATA = {
    "toolchain": {
        "rustc": "rustc fixture",
        "compiler": "Android clang fixture",
        "sdk": "Android NDK fixture; API 26 sysroot",
        "linker": "LLD fixture",
    },
    "provenance": {
        "source_revision": "a" * 40,
        "build_recipe": "scripts/build-android-sdk.sh",
        "build_recipe_sha256": "b" * 64,
        "licenses": ["Apache-2.0", "BSD-3-Clause", "BSL-1.1", "MIT"],
    },
}


class AndroidPackageManifestTest(unittest.TestCase):
    def test_package_version_must_be_safe_semver(self):
        self.assertEqual(gen.package_version("1.2.3-rc-alpha.1+build.7"),
                         "1.2.3-rc-alpha.1+build.7")
        for invalid in ("1", "01.0.0", "1.0.0-", "1.0.0/../../escape"):
            with self.assertRaises(ValueError):
                gen.package_version(invalid)

    def test_sdk_build_verifies_v8_inputs_before_link_and_final_tree_after_manifest(self):
        script = (_MODULE_PATH.parent / "build-android-sdk.sh").read_text()
        metadata_writer = (_MODULE_PATH.parent / "write-android-build-metadata.py").read_text()
        self.assertIn("write-android-build-metadata.py", script)
        self.assertIn("--build-recipe scripts/build-android-sdk.sh", script)
        self.assertIn("--build-metadata", script)
        self.assertIn('"BSL-1.1"', metadata_writer)
        self.assertIn("check-snapshot-freshness.sh", script)
        self.assertIn("snapshot_require_materialized_snapshot", script)
        self.assertIn('--product-profile full --os android "$ARCH"', script)
        self.assertLess(
            script.index("check-snapshot-freshness.sh"),
            script.index("building capi staticlib"),
        )
        self.assertIn("verify-v8-component", script)
        self.assertLess(
            script.index("verify-v8-component"),
            script.index("building capi staticlib"),
        )
        self.assertIn("verify-android-package", script)
        self.assertGreater(
            script.index("verify-android-package"),
            script.index("--v8-component-manifest"),
        )

    def test_snapshot_identity_comes_from_the_verified_snapshot_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            snapshot = root / "SNAPSHOT-full-aarch64.bin"
            snapshot.write_bytes(b"snapshot-bytes")
            snapshot_manifest = root / "snapshot.json"
            value = {
                "schema_version": 3,
                "snapshot_kind": "host",
                "profile": "full",
                "arch": "aarch64",
                "target_triple": "aarch64-linux-android",
                "generation_cpu_policy": "target-baseline",
                "normalized_parameters": [
                    "--arch=aarch64",
                    "--cpu-policy=target-baseline",
                    "--product-profile=full",
                    "--runtime-kind=host",
                    "--warmup=none",
                ],
                "external_references_sha256": "1" * 64,
                "bootstrap_inputs_sha256": "2" * 64,
                "features": ["profile-full"],
                "features_sha256": "3" * 64,
                "rust_sources_sha256": "4" * 64,
                "v8_archive_sha256": "5" * 64,
                "snapshot_size": len(b"snapshot-bytes"),
                "snapshot_sha256": hashlib.sha256(b"snapshot-bytes").hexdigest(),
                "js_sources_sha256": "6" * 64,
                "deno_core_version": "0.385.0",
            }
            snapshot_manifest.write_text(json.dumps(value))

            identity = gen.snapshot_identity(
                snapshot, snapshot_manifest, "aarch64"
            )

        self.assertEqual(identity["schema"], "3")
        self.assertEqual(identity["runtime_kind"], "host")
        self.assertEqual(identity["product_profile"], "full")
        self.assertEqual(identity["bootstrap_inputs_hash"], "2" * 64)
        self.assertEqual(identity["v8_archive_hash"], "5" * 64)
        self.assertEqual(identity["bytes_size"], len(b"snapshot-bytes"))
        self.assertEqual(
            identity["bytes_hash"], hashlib.sha256(b"snapshot-bytes").hexdigest()
        )

    def test_snapshot_manifest_cannot_name_different_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            snapshot = root / "snapshot.bin"
            snapshot.write_bytes(b"actual")
            snapshot_manifest = root / "snapshot.json"
            snapshot_manifest.write_text(
                json.dumps(
                    {
                        "arch": "aarch64",
                        "target_triple": "aarch64-linux-android",
                        "snapshot_size": 6,
                        "snapshot_sha256": "0" * 64,
                    }
                )
            )
            with self.assertRaisesRegex(ValueError, "snapshot_sha256"):
                gen.snapshot_identity(snapshot, snapshot_manifest, "aarch64")

    def test_v8_identity_is_the_complete_component_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "component-manifest.json"
            component = {
                "schema": "migo-v8-component-manifest/v1",
                "component_id": "a" * 64,
                "target": {"triple": "aarch64-linux-android"},
                "runtime": {"v8_revision": "b" * 40},
            }
            path.write_text(json.dumps(component))
            self.assertEqual(gen.v8_identity(path, "aarch64"), component)

    def test_artifacts_carry_size_and_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = pathlib.Path(directory)
            artifact = prefix / "lib/libmigo_capi.a"
            artifact.parent.mkdir(parents=True)
            artifact.write_bytes(b"android-static")
            relative, identity = gen.artifact_identity(prefix, artifact)

        self.assertEqual(relative, "lib/libmigo_capi.a")
        self.assertEqual(identity["size_bytes"], len(b"android-static"))
        self.assertEqual(
            identity["sha256"], hashlib.sha256(b"android-static").hexdigest()
        )

    def test_package_artifacts_cover_every_staged_regular_file(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = pathlib.Path(directory)
            files = {
                "include/migo/migo.h": b"header",
                "lib/cmake/migo/migo-config.cmake": b"cmake",
                "lib/libmigo_capi.a": b"library",
            }
            for relative, contents in files.items():
                path = prefix / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(contents)

            artifacts = gen.package_artifacts(prefix)

        self.assertEqual(set(artifacts), set(files))

    def test_manifest_declares_release_and_codegen_identity(self):
        manifest = gen.render_manifest(
            version="1.0.0",
            arch="aarch64",
            libs=["-lEGL"],
            snapshot={"runtime_kind": "host"},
            v8={"component_id": "a" * 64},
            build_metadata=BUILD_METADATA,
            artifacts={"lib/libmigo_capi.a": {"size_bytes": 1, "sha256": "b" * 64}},
        )
        self.assertEqual(manifest["schema"], "migo-android-package-manifest/v2")
        self.assertEqual(manifest["product_profile"], "full")
        self.assertEqual(manifest["build_type"], "release")
        self.assertEqual(manifest["codegen_profile"], "z")
        self.assertEqual(manifest["toolchain"], BUILD_METADATA["toolchain"])
        self.assertEqual(manifest["graphics"]["backend_family"], "gles-native")
        self.assertIn("BSL-1.1", manifest["provenance"]["licenses"])


if __name__ == "__main__":
    unittest.main()
