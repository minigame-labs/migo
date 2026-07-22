import importlib.util
import pathlib
import subprocess
import tempfile
import unittest


_MODULE_PATH = (
    pathlib.Path(__file__).resolve().parents[1].parent
    / "write-linux-v8-component-manifest.py"
)
_spec = importlib.util.spec_from_file_location("write_linux_v8_component_manifest", _MODULE_PATH)
writer = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(writer)


class LinuxV8ComponentWriterTest(unittest.TestCase):
    def test_declared_patch_must_reproduce_the_exact_worktree_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            source = root / "rusty_v8"
            source.mkdir()

            def git(*arguments):
                return subprocess.run(
                    ["git", "-C", str(source), *arguments],
                    check=True,
                    capture_output=True,
                    text=True,
                )

            git("init", "--quiet")
            git("config", "user.name", "Migo Test")
            git("config", "user.email", "migo-test@example.invalid")
            build_rs = source / "build.rs"
            filler = "".join(f"// stable context {index}\n" for index in range(20))
            build_rs.write_text(
                f"fn main() {{}}\n{filler}fn helper() {{}}\n", encoding="utf-8"
            )
            (source / ".gitignore").write_text("v8/\n", encoding="utf-8")
            git("add", ".gitignore", "build.rs")
            git("commit", "--quiet", "-m", "baseline")

            v8 = source / "v8"
            v8.mkdir()
            subprocess.run(["git", "-C", str(v8), "init", "--quiet"], check=True)
            subprocess.run(
                ["git", "-C", str(v8), "config", "user.name", "Migo Test"],
                check=True,
            )
            subprocess.run(
                [
                    "git", "-C", str(v8), "config", "user.email",
                    "migo-test@example.invalid",
                ],
                check=True,
            )
            (v8 / "README").write_text("v8 baseline\n", encoding="utf-8")
            subprocess.run(["git", "-C", str(v8), "add", "README"], check=True)
            subprocess.run(
                ["git", "-C", str(v8), "commit", "--quiet", "-m", "baseline"],
                check=True,
            )

            declared_source = (
                f"fn main() {{ println!(\"declared\"); }}\n"
                f"{filler}fn helper() {{}}\n"
            )
            build_rs.write_text(declared_source, encoding="utf-8")
            patch = root / "declared.diff"
            patch.write_text(git("diff", "--binary", "HEAD").stdout, encoding="utf-8")

            identities = writer.verify_source_changes(
                source, [("declared", patch)]
            )
            self.assertEqual([identity["id"] for identity in identities], ["declared"])

            build_rs.write_text(
                declared_source.replace("fn helper() {}", "fn helper() { todo!() }"),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(RuntimeError, "exactly reproduce"):
                writer.verify_source_changes(source, [("declared", patch)])

    def test_gn_arguments_are_sorted_and_duplicate_keys_are_rejected(self):
        self.assertEqual(
            writer.normalized_gn_arguments(
                "use_sysroot=true is_official_build=true symbol_level=0"
            ),
            ["is_official_build=true", "symbol_level=0", "use_sysroot=true"],
        )
        with self.assertRaisesRegex(ValueError, "duplicate GN argument key"):
            writer.normalized_gn_arguments("use_sysroot=true use_sysroot=false")

    def test_component_records_linux_floor_and_exact_v8_revisions(self):
        component = writer.build_component(
            rusty_v8_version="145.0.0",
            rusty_v8_revision="a" * 40,
            v8_revision="b" * 40,
            gn_args=["use_sysroot=true"],
            patches=[{"id": "fixture-patch", "sha256": "c" * 64}],
            archive_sha256="d" * 64,
            binding_sha256="e" * 64,
            rustc="rustc fixture",
            compiler="clang fixture",
            sdk="Debian bullseye sysroot fixture",
            linker="LLD fixture",
            recipe_sha256="f" * 64,
        )
        self.assertEqual(component["schema"], "migo-v8-component-manifest/v1")
        self.assertEqual(component["target"]["runtime_floor"]["glibc"], "2.31")
        self.assertEqual(component["target"]["runtime_floor"]["glibcxx"], "3.4.28")
        self.assertEqual(component["runtime"]["rusty_v8_revision"], "a" * 40)
        self.assertEqual(component["runtime"]["v8_revision"], "b" * 40)
        self.assertEqual(component["hashes"]["archive"], "d" * 64)


if __name__ == "__main__":
    unittest.main()
