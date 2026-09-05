"""The artifact half of the Apple deployment-floor gate, over assembled slices.

The rule this pins was wrong once. The gate first required every Mach-O in the
shipped archive to declare the contract's floor exactly, which no Rust + Skia
static archive can satisfy: the device slice carries hundreds of objects from
rustup's prebuilt std at iOS 10.0 and over a thousand from Skia's GN build at
iOS 12.0, and no deployment target this repository sets reaches either. Only a
real build showed it. What matters is the direction -- an object built against
an OLDER target loads under the floor, one built against a NEWER target needs
more OS than the product promises -- plus the proof that something in each slice
was built against the floor at all, without which a build that never received a
deployment target would pass on its dependencies' backwards compatibility.

The gate is run as CI runs it, from the repository root, so its declaration half
runs too; a failure there fails these as well, which is correct -- the same
number is being checked from both ends.
"""

from __future__ import annotations

import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import unittest

_HERE = pathlib.Path(__file__).resolve()
_REPO = _HERE.parents[3]
_GATE = pathlib.Path("scripts/test-apple-deployment-floor-contract.sh")

# The Mach-O, ar and universal writers live with the reader's own tests, which is
# where they are exercised in detail. Reusing them keeps one description of each
# file format in the tree.
_spec = importlib.util.spec_from_file_location(
    "test_macho_build_version", _HERE.parent / "test_macho_build_version.py"
)
fixtures = importlib.util.module_from_spec(_spec)
sys.modules["test_macho_build_version"] = fixtures
_spec.loader.exec_module(fixtures)

_ARM64 = 0x0100000C
_X86_64 = 0x01000007

_IOS = 2
_IOS_SIMULATOR = 7
_MACOS = 1


def obj(minos, platform=_IOS, cputype=_ARM64):
    return fixtures.macho_object(
        (fixtures.build_version(platform=platform, minos=minos),), cputype=cputype
    )


def archive(*members):
    return fixtures.archive(
        fixtures.ar_member("__.SYMDEF", b"\x00" * 8),
        *[fixtures.ar_member(name, data, extended=len(name) > 16) for name, data in members],
    )


class TheArtifactHalf(unittest.TestCase):
    def run_gate(self, slice_name: str, library: bytes):
        with tempfile.TemporaryDirectory() as tmp:
            xcframework = pathlib.Path(tmp) / "MigoEngine.xcframework"
            (xcframework / slice_name).mkdir(parents=True)
            (xcframework / slice_name / "libmigo.a").write_bytes(library)
            (xcframework / "Info.plist").write_text("<plist/>", encoding="utf-8")
            finished = subprocess.run(
                ["bash", str(_GATE), "--artifacts", str(xcframework)],
                cwd=_REPO,
                capture_output=True,
                text=True,
            )
        return finished.returncode, finished.stdout + finished.stderr

    def test_what_a_real_build_produces_passes(self):
        status, output = self.run_gate(
            "ios-arm64",
            archive(
                ("migo_capi.o", obj((15, 0, 0))),
                ("std-0000.std.0000-cgu.0.rcgu.o", obj((10, 0, 0))),
                ("libskparagraph.Decorations.o", obj((12, 0, 0))),
            ),
        )
        self.assertEqual(status, 0, output)
        self.assertIn("was built against the declared ios floor 15.0", output)
        # Below-floor populations must stay visible rather than pass silently.
        self.assertIn("built against ios 10.0, below the 15.0 floor", output)
        self.assertIn("built against ios 12.0, below the 15.0 floor", output)

    def test_an_object_needing_a_newer_os_fails(self):
        status, output = self.run_gate(
            "ios-arm64",
            archive(("migo_capi.o", obj((15, 0, 0))), ("thirdparty.o", obj((17, 0, 0)))),
        )
        self.assertEqual(status, 1, output)
        self.assertIn("built against ios 17.0, newer than the 15.0 floor", output)

    def test_a_patch_component_above_the_floor_fails(self):
        status, output = self.run_gate(
            "ios-arm64",
            archive(("migo_capi.o", obj((15, 0, 0))), ("thirdparty.o", obj((15, 0, 1)))),
        )
        self.assertEqual(status, 1, output)
        self.assertIn("built against ios 15.0.1, newer than the 15.0 floor", output)

    def test_a_build_that_never_received_the_deployment_target_fails(self):
        """Everything below the floor, which "not newer than the floor" alone allows."""
        status, output = self.run_gate(
            "ios-arm64",
            archive(("migo_capi.o", obj((10, 0, 0))), ("std.o", obj((10, 0, 0)))),
        )
        self.assertEqual(status, 1, output)
        self.assertIn("nothing in the arm64 slice was built against the declared floor", output)

    def test_one_slice_of_a_universal_archive_missing_the_floor_fails(self):
        library = fixtures.fat(
            (_ARM64, 0, archive(("migo_capi.o", obj((15, 0, 0), platform=_IOS_SIMULATOR)))),
            (
                _X86_64,
                0,
                archive(
                    (
                        "migo_capi.o",
                        obj((14, 0, 0), platform=_IOS_SIMULATOR, cputype=_X86_64),
                    )
                ),
            ),
        )
        status, output = self.run_gate("ios-arm64_x86_64-simulator", library)
        self.assertEqual(status, 1, output)
        self.assertIn("nothing in the x86_64 slice was built against the declared floor", output)

    def test_the_macos_floor_is_read_from_the_contract_not_the_ios_one(self):
        library = fixtures.fat(
            (
                _ARM64,
                0,
                archive(("migo_capi.o", obj((11, 0, 0), platform=_MACOS))),
            ),
            (
                _X86_64,
                0,
                archive(
                    ("migo_capi.o", obj((11, 0, 0), platform=_MACOS, cputype=_X86_64)),
                    ("std.o", obj((10, 12, 0), platform=_MACOS, cputype=_X86_64)),
                ),
            ),
        )
        status, output = self.run_gate("macos-arm64_x86_64", library)
        self.assertEqual(status, 0, output)
        self.assertIn("was built against the declared macos floor 11.0", output)
        self.assertIn("built against macos 10.12, below the 11.0 floor", output)

    def test_a_platform_the_contract_does_not_cover_fails(self):
        status, output = self.run_gate(
            "ios-arm64", archive(("migo_capi.o", obj((15, 0, 0), platform=6)))
        )
        self.assertEqual(status, 1, output)
        self.assertIn("targets maccatalyst", output)

    def test_an_unreadable_archive_stops_the_gate(self):
        status, output = self.run_gate("ios-arm64", b"!<arch>\n" + b"\x00" * 60)
        self.assertEqual(status, 1, output)
        self.assertIn("could not read the Mach-O files", output)


if __name__ == "__main__":
    unittest.main()
