import importlib.util
import hashlib
import pathlib
import tempfile
import unittest

_MODULE_PATH = (
    pathlib.Path(__file__).resolve().parents[1].parent / "gen-linux-package-metadata.py"
)
_spec = importlib.util.spec_from_file_location("gen_linux_package_metadata", _MODULE_PATH)
gen = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gen)

LINUX_SYSROOT_IDENTITY = (
    "Debian bullseye amd64 sysroot; "
    f"sysroots.json sha256={'c' * 64}"
)
BUILD_METADATA = {
    "toolchain": {
        "rustc": "rustc fixture",
        "compiler": "clang fixture",
        "sdk": LINUX_SYSROOT_IDENTITY,
        "linker": "LLD fixture",
    },
    "provenance": {
        "source_revision": "a" * 40,
        "build_recipe": "scripts/build-linux-sdk.sh",
        "build_recipe_sha256": "b" * 64,
        "licenses": ["Apache-2.0", "BSD-3-Clause", "BSL-1.1", "MIT"],
    },
}


# Captured from `cargo rustc -p migo-capi --lib --crate-type staticlib --
# --print native-static-libs` on 2026-07-19, after the graphics crate began
# declaring -lGL.
CARGO_OUTPUT = """
warning: `capi` (lib) generated 5 warnings
note: native-static-libs: -lGL -lstdc++ -lfreetype -lfontconfig -lEGL -ldl -lasound -lstdc++ -lgcc_s -lutil -lrt -lpthread -lm -ldl -lc
    Finished `release` profile [optimized] target(s) in 6.55s
"""


class ParseNativeStaticLibsTest(unittest.TestCase):
    def test_extracts_the_library_list(self):
        libs = gen.parse_native_static_libs(CARGO_OUTPUT)
        self.assertEqual(libs[0], "-lGL")
        self.assertIn("-lEGL", libs)
        self.assertIn("-lasound", libs)

    def test_deduplicates_while_preserving_first_position(self):
        libs = gen.parse_native_static_libs(CARGO_OUTPUT)
        self.assertEqual(libs.count("-lstdc++"), 1)
        self.assertEqual(libs.count("-ldl"), 1)
        self.assertLess(libs.index("-lstdc++"), libs.index("-lfreetype"))

    def test_missing_note_is_an_error(self):
        # Returning an empty list here would generate a .pc that links nothing
        # and fails at the consumer with undefined symbols, far from the cause.
        with self.assertRaises(ValueError):
            gen.parse_native_static_libs("no note here")


class RenderPkgConfigTest(unittest.TestCase):
    def test_package_version_must_be_safe_semver(self):
        self.assertEqual(gen.package_version("1.2.3-rc-alpha.1+build.7"),
                         "1.2.3-rc-alpha.1+build.7")
        for invalid in ("1", "01.0.0", "1.0.0-", "1.0.0/../../escape"):
            with self.assertRaises(ValueError):
                gen.package_version(invalid)

    def test_description_does_not_claim_one_form(self):
        # Both forms ship in one package and `-lmigo` resolves to the shared one
        # by default, so naming a form in the description would be wrong for
        # whichever half the consumer actually links.
        for shared in (True, False):
            text = gen.render_pkg_config("1.0.0", ["-lGL"], shared=shared)
            self.assertNotIn("static library", text)
            self.assertNotIn("shared library", text)

    def test_static_build_puts_system_libs_in_libs_private(self):
        text = gen.render_pkg_config("1.0.0", ["-lstdc++", "-lGL"], shared=False)
        self.assertIn("Libs: -L${libdir} -lmigo", text)
        self.assertIn("Libs.private: -lstdc++ -lGL", text)
        self.assertIn("Cflags: -I${includedir}", text)
        self.assertIn("Version: 1.0.0", text)

    def test_shared_build_still_declares_private_libs_for_static_linking(self):
        text = gen.render_pkg_config("1.0.0", ["-lGL"], shared=True)
        self.assertIn("Libs: -L${libdir} -lmigo", text)
        self.assertIn("Libs.private: -lGL", text)

    def test_prefix_is_relative_so_the_package_is_relocatable(self):
        text = gen.render_pkg_config("1.0.0", ["-lGL"], shared=False)
        self.assertIn("prefix=${pcfiledir}/../..", text)


class RenderCmakeConfigTest(unittest.TestCase):
    def test_declares_an_imported_target_with_interface_libraries(self):
        text = gen.render_cmake_config("1.0.0", ["-lstdc++", "-lGL"], shared=False)
        self.assertIn("add_library(migo::migo STATIC IMPORTED)", text)
        self.assertIn("INTERFACE_LINK_LIBRARIES", text)
        self.assertIn("stdc++;GL", text)

    def test_shared_form_imports_a_shared_library(self):
        text = gen.render_cmake_config("1.0.0", ["-lGL"], shared=True)
        self.assertIn("add_library(migo::migo SHARED IMPORTED)", text)
        self.assertIn("libmigo.so", text)

    def test_shared_form_does_not_propagate_system_libraries(self):
        # libmigo.so carries them in DT_NEEDED. Propagating them would force
        # every consumer to install the -dev package of GL, EGL, fontconfig and
        # freetype just to link -- measured: the CMake example failed on exactly
        # those four before this.
        text = gen.render_cmake_config("1.0.0", ["-lGL", "-lEGL"], shared=True)
        self.assertIn('INTERFACE_LINK_LIBRARIES ""', text)


class RenderManifestTest(unittest.TestCase):
    def test_sdk_build_verifies_v8_inputs_before_link_and_final_tree_after_manifest(self):
        script = (_MODULE_PATH.parent / "build-linux-sdk.sh").read_text()
        sysroot_helper = (_MODULE_PATH.parent / "lib/linux-sysroot.sh").read_text()
        metadata_writer_path = _MODULE_PATH.parent / "write-linux-build-metadata.py"
        self.assertTrue(metadata_writer_path.is_file())
        metadata_writer = metadata_writer_path.read_text()
        self.assertIn("write-linux-build-metadata.py", script)
        self.assertIn("--build-metadata", script)
        self.assertIn('"BSL-1.1"', metadata_writer)
        self.assertIn('--sysroot "$MIGO_SYSROOT_IDENTITY"', script)
        self.assertNotIn('--sysroot "$MIGO_SYSROOT"', script)
        self.assertIn("sysroots.json sha256=", sysroot_helper)
        self.assertNotIn("/home/xg/", sysroot_helper)
        self.assertIn("V8 component sysroot identity does not match", script)
        self.assertLess(
            script.index("V8 component sysroot identity does not match"),
            script.index("building capi staticlib"),
        )
        self.assertIn("verify-v8-component", script)
        self.assertLess(
            script.index("verify-v8-component"),
            script.index("building capi staticlib"),
        )
        self.assertIn("verify-linux-package", script)
        self.assertGreater(
            script.index("verify-linux-package"),
            script.index("--v8-component-manifest"),
        )

    def test_records_the_contract_fields(self):
        artifact = {
            "size_bytes": 123456,
            "sha256": "a" * 64,
        }
        manifest = gen.render_manifest(
            version="1.0.0",
            needed=["libEGL.so.1", "libc.so.6"],
            v8={"schema": "migo-v8-component-manifest/v1", "component_id": "b" * 64},
            sysroot=LINUX_SYSROOT_IDENTITY,
            build_metadata=BUILD_METADATA,
            artifacts={"lib/libmigo.a": artifact},
        )
        self.assertEqual(manifest["schema"], "migo-linux-package-manifest/v2")
        self.assertEqual(manifest["product_profile"], "full")
        self.assertEqual(manifest["build_type"], "release")
        self.assertEqual(manifest["codegen_profile"], "z")
        self.assertEqual(manifest["target"], "x86_64-unknown-linux-gnu")
        self.assertEqual(manifest["cpu_baseline"], "x86-64-v1")
        self.assertEqual(manifest["glibc_floor"], "2.31")
        self.assertEqual(manifest["glibcxx_floor"], "3.4.28")
        self.assertEqual(manifest["dynamic_dependencies"], ["libEGL.so.1", "libc.so.6"])
        self.assertEqual(manifest["v8"]["component_id"], "b" * 64)
        self.assertEqual(manifest["artifacts"]["lib/libmigo.a"], artifact)
        self.assertEqual(manifest["toolchain"], BUILD_METADATA["toolchain"])
        self.assertEqual(manifest["graphics"]["backend_family"], "gles-native")
        self.assertIn("BSL-1.1", manifest["provenance"]["licenses"])

    def test_dependencies_are_sorted_for_stable_diffs(self):
        manifest = gen.render_manifest(
            version="1.0.0", needed=["libc.so.6", "libEGL.so.1"], v8={},
            sysroot="s", build_metadata=BUILD_METADATA, artifacts={})
        self.assertEqual(manifest["dynamic_dependencies"], ["libEGL.so.1", "libc.so.6"])

    def test_artifact_identity_hashes_the_staged_regular_file(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = pathlib.Path(directory)
            artifact = prefix / "lib/libmigo.a"
            artifact.parent.mkdir(parents=True)
            artifact.write_bytes(b"migo-static")
            relative, identity = gen.artifact_identity(prefix, artifact)

        self.assertEqual(relative, "lib/libmigo.a")
        self.assertEqual(identity["size_bytes"], len(b"migo-static"))
        self.assertEqual(identity["sha256"], hashlib.sha256(b"migo-static").hexdigest())

    def test_package_artifacts_cover_headers_and_integration_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = pathlib.Path(directory)
            files = {
                "include/migo/migo.h": b"header",
                "lib/cmake/migo/migo-config.cmake": b"cmake",
                "lib/libmigo.a": b"archive",
                "lib/libmigo.so.0.1.0": b"shared",
                "lib/pkgconfig/migo.pc": b"pkg-config",
            }
            for relative, contents in files.items():
                path = prefix / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(contents)
            (prefix / "lib/libmigo.so").symlink_to("libmigo.so.1")

            artifacts = gen.package_artifacts(prefix)

        self.assertEqual(set(artifacts), set(files))


if __name__ == "__main__":
    unittest.main()
