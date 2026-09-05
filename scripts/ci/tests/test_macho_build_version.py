"""The Mach-O reader behind the Apple deployment-floor contract's artifact half.

The fixtures are assembled here rather than captured from a build, and that is
the opposite of what this repository's other binary-audit test does on purpose.
`test_abi_floor_audit.py` pins objdump's *text* output, where a hand-written
fixture would encode a guess about a tool's formatting; these fixtures are the
file formats themselves, frozen in <mach-o/loader.h> and <mach-o/fat.h>, and
assembling them is the only way to cover the cases a real toolchain will not
produce on demand -- an object carrying no deployment target, a universal header
naming one architecture twice, a member whose size runs past the end.

The reader was also run against real Apple objects cross-compiled on a Linux
host (`clang --target=arm64-apple-ios15.0`, `llvm-ar`, `llvm-lipo`) before it
was believed.
"""

from __future__ import annotations

import importlib.util
import io
import pathlib
import struct
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

_MODULE_PATH = pathlib.Path(__file__).resolve().parents[1].parent / "lib" / "macho_build_version.py"
_spec = importlib.util.spec_from_file_location("macho_build_version", _MODULE_PATH)
macho = importlib.util.module_from_spec(_spec)
# Registered before execution because the module's dataclasses resolve their
# string annotations through sys.modules, and a module loaded by spec alone is
# not there yet: without this line the import fails inside @dataclass.
sys.modules["macho_build_version"] = macho
_spec.loader.exec_module(macho)

_ARM64 = 0x0100000C
_X86_64 = 0x01000007

_MAGICS = {
    ("<", 64): b"\xcf\xfa\xed\xfe",
    (">", 64): b"\xfe\xed\xfa\xcf",
    ("<", 32): b"\xce\xfa\xed\xfe",
    (">", 32): b"\xfe\xed\xfa\xce",
}


def _packed(version: tuple[int, int, int]) -> int:
    major, minor, patch = version
    return (major << 16) | (minor << 8) | patch


def build_version(platform: int = 2, minos=(15, 0, 0), endian: str = "<") -> bytes:
    return struct.pack(
        endian + "6I", 0x32, 24, platform, _packed(minos), _packed((18, 5, 0)), 0
    )


def version_min(cmd: int = 0x25, version=(15, 0, 0), endian: str = "<") -> bytes:
    return struct.pack(endian + "4I", cmd, 16, _packed(version), _packed(version))


def symtab(endian: str = "<") -> bytes:
    """A load command the reader must walk past without noticing it."""
    return struct.pack(endian + "6I", 0x02, 24, 0, 0, 0, 0)


def macho_object(
    commands: tuple[bytes, ...] = (),
    cputype: int = _ARM64,
    cpusubtype: int = 0,
    endian: str = "<",
    bits: int = 64,
) -> bytes:
    body = b"".join(commands)
    header = _MAGICS[(endian, bits)] + struct.pack(
        endian + "6I", cputype, cpusubtype, 1, len(commands), len(body), 0
    )
    if bits == 64:
        header += struct.pack(endian + "I", 0)
    return header + body


def ar_member(name: str, data: bytes, extended: bool = False) -> bytes:
    if extended:
        # The BSD long-name form: the name occupies the head of the member and
        # is counted in its declared size. Apple's toolchain pads it with NULs
        # to align the object, which the reader must strip.
        payload = name.encode("utf-8")
        payload += b"\x00" * ((8 - len(payload) % 8) % 8)
        field = f"#1/{len(payload)}"
        body = payload + data
    else:
        # Refused rather than truncated: a name that overflows the 16-byte field
        # runs into the header's mtime and yields an archive no reader can parse.
        # A builder that emits one quietly turns a passing test into a lie about
        # the parser.
        if len(name) > 16:
            raise ValueError(f"{name!r} exceeds ar's 16-byte name field; pass extended=True")
        field = name
        body = data
    header = (
        field.ljust(16)
        + "0".ljust(12)
        + "0".ljust(6)
        + "0".ljust(6)
        + "100644".ljust(8)
        + str(len(body)).ljust(10)
        + "`\n"
    )
    out = header.encode("ascii") + body
    if len(body) % 2:
        out += b"\n"
    return out


def archive(*members: bytes) -> bytes:
    return b"!<arch>\n" + b"".join(members)


def fat(*slices: tuple[int, int, bytes], bits: int = 32) -> bytes:
    entry_size = 32 if bits == 64 else 20
    magic = b"\xca\xfe\xba\xbf" if bits == 64 else b"\xca\xfe\xba\xbe"
    header = magic + struct.pack(">I", len(slices))
    offset = 8 + entry_size * len(slices)
    table, body = b"", b""
    for cputype, cpusubtype, payload in slices:
        if bits == 64:
            table += struct.pack(">iiQQII", cputype, cpusubtype, offset, len(payload), 0, 0)
        else:
            table += struct.pack(">iiIII", cputype, cpusubtype, offset, len(payload), 0)
        body += payload
        offset += len(payload)
    return header + table + body


class ReadsTheDeclaredTarget(unittest.TestCase):
    def test_bare_object(self):
        report = macho.read_report(macho_object((build_version(),)), "t.o")
        self.assertEqual(report.machos, 1)
        (record,) = report.records
        self.assertEqual(
            (record.arch, record.platform, record.minos, record.load_command),
            ("arm64", "ios", "15.0", "LC_BUILD_VERSION"),
        )

    def test_walks_past_other_load_commands(self):
        report = macho.read_report(
            macho_object((symtab(), build_version(), symtab())), "t.o"
        )
        self.assertEqual([record.minos for record in report.records], ["15.0"])

    def test_big_endian_and_32_bit(self):
        for endian in ("<", ">"):
            for bits in (32, 64):
                with self.subTest(endian=endian, bits=bits):
                    report = macho.read_report(
                        macho_object(
                            (build_version(platform=1, minos=(11, 0, 0), endian=endian),),
                            endian=endian,
                            bits=bits,
                        ),
                        "t.o",
                    )
                    (record,) = report.records
                    self.assertEqual((record.platform, record.minos), ("macos", "11.0"))

    def test_platform_and_architecture_names(self):
        cases = {
            (1, _ARM64, 0): ("macos", "arm64"),
            (2, _ARM64, 0): ("ios", "arm64"),
            (7, _X86_64, 0): ("iossimulator", "x86_64"),
            (6, _ARM64, 0): ("maccatalyst", "arm64"),
            (2, _ARM64, 2): ("ios", "arm64e"),
        }
        for (platform, cputype, cpusubtype), expected in cases.items():
            with self.subTest(platform=platform, cputype=cputype):
                report = macho.read_report(
                    macho_object(
                        (build_version(platform=platform),),
                        cputype=cputype,
                        cpusubtype=cpusubtype,
                    ),
                    "t.o",
                )
                (record,) = report.records
                self.assertEqual((record.platform, record.arch), expected)

    def test_unknown_platform_is_reported_not_dropped(self):
        report = macho.read_report(macho_object((build_version(platform=99),)), "t.o")
        (record,) = report.records
        self.assertEqual(record.platform, "platform-99")

    def test_patch_component_is_kept_visible(self):
        report = macho.read_report(macho_object((build_version(minos=(15, 0, 1)),)), "t.o")
        (record,) = report.records
        self.assertEqual(record.minos, "15.0.1")

    def test_superseded_version_min_command(self):
        report = macho.read_report(macho_object((version_min(),)), "t.o")
        (record,) = report.records
        self.assertEqual(
            (record.platform, record.minos, record.load_command),
            ("ios", "15.0", "LC_VERSION_MIN_IPHONEOS"),
        )

    def test_object_with_no_deployment_target(self):
        """The case a real toolchain will not produce, and the one the gate needs.

        An architecture whose objects declare nothing must be visible as a Mach-O
        that was read and yielded no record, not as an absence.
        """
        report = macho.read_report(macho_object((symtab(),)), "t.o")
        self.assertEqual((report.machos, report.records), (1, []))
        self.assertEqual(report.architectures["arm64"].machos, 1)

    def test_not_a_macho_container(self):
        self.assertIsNone(macho.read_report(b"#ifndef MIGO_H\n", "migo.h"))
        self.assertIsNone(macho.read_report(b"", "empty"))


class ReadsEveryArchiveMember(unittest.TestCase):
    def test_skips_the_symbol_table_and_reads_both_objects(self):
        data = archive(
            ar_member("__.SYMDEF", b"\x00" * 16),
            ar_member("a.o", macho_object((build_version(),))),
            ar_member("b.o", macho_object((build_version(minos=(15, 2, 0)),))),
        )
        report = macho.read_report(data, "lib.a")
        self.assertEqual(report.machos, 2)
        self.assertEqual(
            sorted((record.member, record.minos) for record in report.records),
            [("a.o", "15.0"), ("b.o", "15.2")],
        )

    def test_extended_member_names(self):
        name = "a_rather_long_object_file_name.o"
        data = archive(ar_member(name, macho_object((build_version(),)), extended=True))
        report = macho.read_report(data, "lib.a")
        (record,) = report.records
        self.assertEqual(record.member, name)

    def test_odd_sized_members_are_padded(self):
        data = archive(
            ar_member("odd.txt", b"x" * 7),
            ar_member("a.o", macho_object((build_version(),))),
        )
        report = macho.read_report(data, "lib.a")
        self.assertEqual([record.member for record in report.records], ["a.o"])

    def test_archive_with_no_objects_reports_none(self):
        data = archive(ar_member("__.SYMDEF", b"\x00" * 8))
        report = macho.read_report(data, "lib.a")
        self.assertEqual((report.machos, report.records), (0, []))


class ReadsEveryUniversalSlice(unittest.TestCase):
    def test_both_slices_of_a_universal_archive(self):
        """The defect that made this reader necessary.

        `otool -l` handed a universal archive prints the host architecture and
        nothing else, so two of the three slice groups this repository ships
        were being audited by half.
        """
        data = fat(
            (_ARM64, 0, archive(ar_member("a.o", macho_object((build_version(platform=7),))))),
            (
                _X86_64,
                0,
                archive(
                    ar_member(
                        "b.o",
                        macho_object((build_version(platform=7),), cputype=_X86_64),
                    )
                ),
            ),
        )
        report = macho.read_report(data, "lib.a")
        self.assertEqual(sorted(report.architectures), ["arm64", "x86_64"])
        self.assertEqual(
            sorted((record.arch, record.minos) for record in report.records),
            [("arm64", "15.0"), ("x86_64", "15.0")],
        )

    def test_slice_disagreeing_with_its_neighbour_is_reported_per_slice(self):
        data = fat(
            (_ARM64, 0, macho_object((build_version(),))),
            (_X86_64, 0, macho_object((build_version(minos=(17, 0, 0)),), cputype=_X86_64)),
        )
        report = macho.read_report(data, "lib.a")
        self.assertEqual(
            sorted((record.arch, record.minos) for record in report.records),
            [("arm64", "15.0"), ("x86_64", "17.0")],
        )

    def test_slice_with_no_deployment_target_stays_visible(self):
        data = fat(
            (_ARM64, 0, macho_object((build_version(),))),
            (_X86_64, 0, macho_object((symtab(),), cputype=_X86_64)),
        )
        report = macho.read_report(data, "lib.a")
        self.assertEqual(report.architectures["x86_64"].machos, 1)
        self.assertEqual(report.architectures["x86_64"].records, [])

    def test_64_bit_universal_header(self):
        data = fat((_ARM64, 0, macho_object((build_version(),))), bits=64)
        report = macho.read_report(data, "lib.a")
        self.assertEqual([record.arch for record in report.records], ["arm64"])

    def test_the_architecture_label_comes_from_the_universal_header(self):
        """A slice is what the header says it is; lipo and xcodebuild read that."""
        data = fat((_X86_64, 0, macho_object((build_version(),), cputype=_ARM64)))
        report = macho.read_report(data, "lib.a")
        self.assertEqual(sorted(report.architectures), ["x86_64"])


class FailsClosed(unittest.TestCase):
    def assertRejects(self, data: bytes, fragment: str):
        with self.assertRaises(macho.MachOParseError) as caught:
            macho.read_report(data, "artifact")
        self.assertIn(fragment, str(caught.exception))

    def test_truncated_header(self):
        self.assertRejects(macho_object((build_version(),))[:20], "header is truncated")

    def test_load_commands_past_the_end(self):
        data = bytearray(macho_object((build_version(),)))
        data[20:24] = struct.pack("<I", 4096)
        self.assertRejects(bytes(data), "run past the end")

    def test_load_command_table_shorter_than_ncmds(self):
        data = bytearray(macho_object((build_version(),)))
        data[16:20] = struct.pack("<I", 2)
        self.assertRejects(bytes(data), "load command table is truncated")

    def test_load_command_size_of_zero(self):
        data = bytearray(macho_object((build_version(),)))
        data[36:40] = struct.pack("<I", 0)
        self.assertRejects(bytes(data), "declares size 0")

    def test_truncated_build_version(self):
        short = struct.pack("<4I", 0x32, 16, 2, 0)
        data = _MAGICS[("<", 64)] + struct.pack("<6I", _ARM64, 0, 1, 1, len(short), 0)
        data += struct.pack("<I", 0) + short
        self.assertRejects(data, "LC_BUILD_VERSION is truncated")

    def test_archive_member_header_without_its_magic(self):
        data = bytearray(archive(ar_member("a.o", macho_object((build_version(),)))))
        data[8 + 58 : 8 + 60] = b"XX"
        self.assertRejects(bytes(data), "member header is malformed")

    def test_archive_member_size_that_is_not_a_number(self):
        data = bytearray(archive(ar_member("a.o", macho_object((build_version(),)))))
        data[8 + 48 : 8 + 58] = b"abcdefghij"
        self.assertRejects(bytes(data), "no readable size")

    def test_archive_member_running_past_the_end(self):
        data = bytearray(archive(ar_member("a.o", macho_object((build_version(),)))))
        data[8 + 48 : 8 + 58] = b"999999    "
        self.assertRejects(bytes(data), "extends past the end")

    def test_extended_name_longer_than_its_member(self):
        data = bytearray(
            archive(ar_member("a.o", macho_object((build_version(),)), extended=True))
        )
        data[8 : 8 + 16] = b"#1/9999         "
        self.assertRejects(bytes(data), "longer than the member")

    def test_universal_slice_past_the_end(self):
        data = bytearray(fat((_ARM64, 0, macho_object((build_version(),)))))
        data[8 + 8 : 8 + 12] = struct.pack(">I", 1 << 20)
        self.assertRejects(bytes(data), "past the end of the file")

    def test_universal_header_claiming_more_slices_than_fit(self):
        data = bytearray(fat((_ARM64, 0, macho_object((build_version(),)))))
        data[4:8] = struct.pack(">I", 4096)
        self.assertRejects(bytes(data), "do not fit in the file")

    def test_universal_header_naming_one_architecture_twice(self):
        data = fat(
            (_ARM64, 0, macho_object((build_version(),))),
            (_ARM64, 0, macho_object((build_version(),))),
        )
        self.assertRejects(data, "twice")

    def test_universal_slice_that_is_neither_object_nor_archive(self):
        self.assertRejects(fat((_ARM64, 0, b"not a mach-o at all")), "neither a Mach-O nor an archive")


class ThePrintedContract(unittest.TestCase):
    """The gate parses these lines by position; changing them breaks it silently."""

    def test_lines_are_grouped_deduplicated_and_relative(self):
        data = fat(
            (
                _ARM64,
                0,
                archive(
                    ar_member("__.SYMDEF", b"\x00" * 8),
                    ar_member("a.o", macho_object((build_version(),))),
                    ar_member("b.o", macho_object((build_version(),))),
                    ar_member("c.o", macho_object((build_version(minos=(15, 2, 0)),))),
                ),
            ),
            (_X86_64, 0, macho_object((build_version(),), cputype=_X86_64)),
        )
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            (root / "ios-arm64").mkdir()
            (root / "ios-arm64" / "libmigo.a").write_bytes(data)
            (root / "Info.plist").write_text("<plist/>", encoding="utf-8")
            printed = io.StringIO()
            with redirect_stdout(printed):
                status = macho.main(["--relative-to", str(root), str(root)])

        self.assertEqual(status, 0)
        self.assertEqual(
            printed.getvalue().splitlines(),
            [
                "FILE\tios-arm64/libmigo.a\t4\t4",
                "ARCH\tios-arm64/libmigo.a\tarm64\t3\t3",
                "RECORD\tios-arm64/libmigo.a\tarm64\tios\t15.0\t2\ta.o",
                "RECORD\tios-arm64/libmigo.a\tarm64\tios\t15.2\t1\tc.o",
                "ARCH\tios-arm64/libmigo.a\tx86_64\t1\t1",
                "RECORD\tios-arm64/libmigo.a\tx86_64\tios\t15.0\t1\t-",
            ],
        )

    def test_an_unreadable_artifact_exits_nonzero(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            (root / "libmigo.a").write_bytes(b"!<arch>\n" + b"\x00" * 60)
            complaint = io.StringIO()
            with redirect_stderr(complaint):
                status = macho.main([str(root)])
        self.assertEqual(status, 1)
        self.assertIn("member header is malformed", complaint.getvalue())


if __name__ == "__main__":
    unittest.main()
