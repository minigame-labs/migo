import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest


_SCRIPT = pathlib.Path(__file__).resolve().parents[1].parent / "write-linux-build-metadata.py"


class LinuxBuildMetadataTest(unittest.TestCase):
    def test_writer_records_relocatable_toolchain_and_current_license(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            recipe = root / "scripts/build-linux-sdk.sh"
            recipe.parent.mkdir(parents=True)
            recipe.write_text("#!/usr/bin/env bash\n", encoding="utf-8")
            tool = root / "version-tool"
            tool.write_text("#!/usr/bin/env sh\nprintf 'fixture tool 1.0\\n'\n", encoding="utf-8")
            tool.chmod(0o755)
            output = root / "build-metadata.json"
            sysroot_identity = (
                "Debian bullseye amd64 sysroot; "
                f"sysroots.json sha256={'a' * 64}"
            )

            result = subprocess.run(
                [
                    sys.executable,
                    str(_SCRIPT),
                    "--repo-root",
                    str(root),
                    "--output",
                    str(output),
                    "--compiler",
                    str(tool),
                    "--linker",
                    str(tool),
                    "--sysroot-identity",
                    sysroot_identity,
                    "--source-revision",
                    "b" * 40,
                ],
                check=False,
                text=True,
                capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            metadata = json.loads(output.read_text(encoding="utf-8"))

        self.assertEqual(metadata["schema"], "migo-linux-build-metadata/v1")
        self.assertEqual(metadata["toolchain"]["sdk"], sysroot_identity)
        self.assertEqual(metadata["toolchain"]["compiler"], "fixture tool 1.0")
        self.assertEqual(metadata["provenance"]["source_revision"], "b" * 40)
        self.assertEqual(
            metadata["provenance"]["build_recipe_sha256"],
            hashlib.sha256(b"#!/usr/bin/env bash\n").hexdigest(),
        )
        self.assertIn("BSL-1.1", metadata["provenance"]["licenses"])


if __name__ == "__main__":
    unittest.main()
