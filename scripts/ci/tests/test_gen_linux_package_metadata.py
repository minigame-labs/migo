import importlib.util
import pathlib
import unittest

_MODULE_PATH = (
    pathlib.Path(__file__).resolve().parents[1].parent / "gen-linux-package-metadata.py"
)
_spec = importlib.util.spec_from_file_location("gen_linux_package_metadata", _MODULE_PATH)
gen = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gen)


# Captured from `cargo rustc -p capi --lib --crate-type staticlib --
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
    def test_records_the_contract_fields(self):
        manifest = gen.render_manifest(
            version="1.0.0",
            needed=["libEGL.so.1", "libc.so.6"],
            v8={"revision": "145.0.0", "gn_args": ["is_official_build=true"]},
            sysroot="/path/to/debian_bullseye_amd64-sysroot",
            artifacts={"libmigo.a": 123456},
        )
        self.assertEqual(manifest["target"], "x86_64-unknown-linux-gnu")
        self.assertEqual(manifest["cpu_baseline"], "x86-64-v1")
        self.assertEqual(manifest["glibc_floor"], "2.31")
        self.assertEqual(manifest["glibcxx_floor"], "3.4.28")
        self.assertEqual(manifest["dynamic_dependencies"], ["libEGL.so.1", "libc.so.6"])
        self.assertEqual(manifest["v8"]["revision"], "145.0.0")
        self.assertEqual(manifest["artifacts"]["libmigo.a"], 123456)

    def test_dependencies_are_sorted_for_stable_diffs(self):
        manifest = gen.render_manifest(
            version="1.0.0", needed=["libc.so.6", "libEGL.so.1"], v8={},
            sysroot="s", artifacts={})
        self.assertEqual(manifest["dynamic_dependencies"], ["libEGL.so.1", "libc.so.6"])


if __name__ == "__main__":
    unittest.main()
