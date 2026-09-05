#!/usr/bin/env python3
"""Report the Apple deployment target recorded in every Mach-O of an artifact.

The declaration half of the deployment-floor contract reads source. This reads
the bytes that ship, which is the only place a floor can be wrong in a way no
manifest shows.

WHY THIS PARSES THE FILE INSTEAD OF ASKING XCODE'S TOOLS.

Three tools look like they answer this and none of them does:

  * `vtool -show-build` takes one Mach-O or one universal file. Every Apple
    slice this repository ships is an `ar` archive of Mach-O members, and vtool
    reports no build version for one at all.
  * `otool -l` does read archives, but handed a *universal* archive it prints
    one architecture -- the host's -- and `-arch` does not move it. Two of the
    three slice groups are `lipo` output, so that reads half the product and
    says nothing about the other half. A gate that silently audits one slice is
    worse than no gate.
  * `lipo -archs`, which would at least enumerate what to audit, segfaults on a
    thin archive in the LLVM implementation.

The formats here are small, frozen, and documented in <mach-o/loader.h> and
<mach-o/fat.h>. Reading them directly costs less than working around three
tools' quirks, is testable without a Mac, and cannot quietly skip a slice --
every architecture in the fat header and every member in every archive is
visited or the read fails.

Policy lives in the caller. This prints what the bytes say; the floor they are
compared against belongs to contracts/apple/deployment-floor.json and the gate
that reads it.

Malformed input raises rather than returning nothing, because nothing read is
indistinguishable from nothing wrong.
"""

from __future__ import annotations

import argparse
import struct
import sys
from dataclasses import dataclass, field
from pathlib import Path

_AR_MAGIC = b"!<arch>\n"
_AR_HEADER_SIZE = 60

# Universal headers are big-endian on disk; the byte-swapped magics exist and
# are accepted so a swapped file is read rather than mistaken for something
# this tool does not own.
_FAT_MAGICS = {
    b"\xca\xfe\xba\xbe": (">", False),
    b"\xbe\xba\xfe\xca": ("<", False),
    b"\xca\xfe\xba\xbf": (">", True),
    b"\xbf\xba\xfe\xca": ("<", True),
}

_MACHO_MAGICS = {
    b"\xce\xfa\xed\xfe": ("<", False),
    b"\xfe\xed\xfa\xce": (">", False),
    b"\xcf\xfa\xed\xfe": ("<", True),
    b"\xfe\xed\xfa\xcf": (">", True),
}

_LC_BUILD_VERSION = 0x32

# The load commands LC_BUILD_VERSION replaced. Nothing built against the floors
# in this repository emits one -- the newer command covers iOS 12 and macOS
# 10.14 upward -- and they are read anyway so that a file carrying one is
# diagnosed instead of reported as carrying no deployment target at all.
_LC_VERSION_MIN = {
    0x24: ("macos", "LC_VERSION_MIN_MACOSX"),
    0x25: ("ios", "LC_VERSION_MIN_IPHONEOS"),
    0x2F: ("tvos", "LC_VERSION_MIN_TVOS"),
    0x30: ("watchos", "LC_VERSION_MIN_WATCHOS"),
}

_PLATFORMS = {
    1: "macos",
    2: "ios",
    3: "tvos",
    4: "watchos",
    5: "bridgeos",
    6: "maccatalyst",
    7: "iossimulator",
    8: "tvossimulator",
    9: "watchossimulator",
    10: "driverkit",
    11: "visionos",
    12: "visionossimulator",
}

_CPU_TYPES = {
    7: "i386",
    0x01000007: "x86_64",
    12: "arm",
    0x0100000C: "arm64",
    0x0200000C: "arm64_32",
    18: "ppc",
    0x01000012: "ppc64",
}

_CPU_TYPE_ARM64 = 0x0100000C
_CPU_SUBTYPE_ARM64E = 2
_CPU_SUBTYPE_MASK = 0x00FFFFFF


class MachOParseError(RuntimeError):
    """The artifact could not be read, so its floor is unknown rather than fine."""


@dataclass(frozen=True)
class VersionRecord:
    arch: str
    platform: str
    minos: str
    member: str
    load_command: str


@dataclass
class ArchitectureReport:
    arch: str
    machos: int = 0
    records: list[VersionRecord] = field(default_factory=list)


@dataclass
class FileReport:
    label: str
    architectures: dict[str, ArchitectureReport] = field(default_factory=dict)

    @property
    def machos(self) -> int:
        return sum(entry.machos for entry in self.architectures.values())

    @property
    def records(self) -> list[VersionRecord]:
        found: list[VersionRecord] = []
        for entry in self.architectures.values():
            found.extend(entry.records)
        return found

    def entry_for(self, arch: str) -> ArchitectureReport:
        if arch not in self.architectures:
            self.architectures[arch] = ArchitectureReport(arch=arch)
        return self.architectures[arch]


def _format_version(value: int) -> str:
    """xxxx.yy.zz packed into a uint32, printed the way the contract writes it."""
    major, minor, patch = value >> 16, (value >> 8) & 0xFF, value & 0xFF
    if patch:
        return f"{major}.{minor}.{patch}"
    return f"{major}.{minor}"


def _arch_name(cputype: int, cpusubtype: int) -> str:
    if cputype == _CPU_TYPE_ARM64 and (cpusubtype & _CPU_SUBTYPE_MASK) == _CPU_SUBTYPE_ARM64E:
        return "arm64e"
    return _CPU_TYPES.get(cputype, f"cputype-{cputype:#x}")


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise MachOParseError(message)


def _parse_macho(
    data: bytes, start: int, size: int, member: str, report: FileReport, arch_label: str | None
) -> None:
    endian, is_64 = _MACHO_MAGICS[data[start : start + 4]]
    header_size = 32 if is_64 else 28
    _require(size >= header_size, f"{member or 'file'}: Mach-O header is truncated")
    cputype, cpusubtype, _filetype, ncmds, sizeofcmds, _flags = struct.unpack_from(
        endian + "6I", data, start + 4
    )
    _require(
        header_size + sizeofcmds <= size,
        f"{member or 'file'}: load commands run past the end of the Mach-O",
    )

    arch = arch_label or _arch_name(cputype, cpusubtype)
    entry = report.entry_for(arch)
    entry.machos += 1

    cursor = start + header_size
    end = cursor + sizeofcmds
    for _ in range(ncmds):
        _require(cursor + 8 <= end, f"{member or 'file'}: load command table is truncated")
        cmd, cmdsize = struct.unpack_from(endian + "2I", data, cursor)
        _require(
            8 <= cmdsize <= end - cursor,
            f"{member or 'file'}: load command {cmd:#x} declares size {cmdsize}",
        )
        if cmd == _LC_BUILD_VERSION:
            _require(cmdsize >= 24, f"{member or 'file'}: LC_BUILD_VERSION is truncated")
            platform_id, minos = struct.unpack_from(endian + "2I", data, cursor + 8)
            entry.records.append(
                VersionRecord(
                    arch=arch,
                    platform=_PLATFORMS.get(platform_id, f"platform-{platform_id}"),
                    minos=_format_version(minos),
                    member=member,
                    load_command="LC_BUILD_VERSION",
                )
            )
        elif cmd in _LC_VERSION_MIN:
            _require(cmdsize >= 16, f"{member or 'file'}: version-min command is truncated")
            platform, name = _LC_VERSION_MIN[cmd]
            (version,) = struct.unpack_from(endian + "I", data, cursor + 8)
            entry.records.append(
                VersionRecord(
                    arch=arch,
                    platform=platform,
                    minos=_format_version(version),
                    member=member,
                    load_command=name,
                )
            )
        cursor += cmdsize


def _parse_archive(
    data: bytes, start: int, size: int, report: FileReport, arch_label: str | None
) -> None:
    cursor = start + len(_AR_MAGIC)
    limit = start + size
    while cursor + _AR_HEADER_SIZE <= limit:
        header = data[cursor : cursor + _AR_HEADER_SIZE]
        _require(header[58:60] == b"`\n", "archive member header is malformed")
        try:
            declared_size = int(header[48:58].decode("ascii").strip())
        except (UnicodeDecodeError, ValueError) as error:
            raise MachOParseError(f"archive member declares no readable size: {error}") from error
        _require(declared_size >= 0, "archive member declares a negative size")

        body = cursor + _AR_HEADER_SIZE
        _require(
            body + declared_size <= limit, "archive member extends past the end of the archive"
        )

        content, content_size = body, declared_size
        raw_name = header[0:16]
        if raw_name.startswith(b"#1/"):
            # The BSD long-name form Apple's toolchain emits: the name lives in
            # the first bytes of the member and is counted in its size.
            try:
                name_length = int(raw_name[3:].decode("ascii").strip())
            except (UnicodeDecodeError, ValueError) as error:
                raise MachOParseError(
                    f"archive member declares no readable name length: {error}"
                ) from error
            _require(
                0 <= name_length <= declared_size,
                "archive member's extended name is longer than the member",
            )
            name = data[body : body + name_length].rstrip(b"\x00").decode("utf-8", "replace")
            content += name_length
            content_size -= name_length
        else:
            name = raw_name.decode("utf-8", "replace").strip().rstrip("/")

        # Members that are not Mach-O are skipped rather than rejected: an
        # archive legitimately carries a symbol table, and Rust archives carry
        # metadata members. A file whose members are ALL skipped reports zero
        # Mach-O objects, which the caller treats as a failure.
        if content_size >= 4 and data[content : content + 4] in _MACHO_MAGICS:
            _parse_macho(data, content, content_size, name, report, arch_label)

        cursor = body + declared_size
        if declared_size % 2:
            cursor += 1


def _parse_container(
    data: bytes, start: int, size: int, report: FileReport, arch_label: str | None
) -> None:
    head = data[start : start + 4]
    if head in _MACHO_MAGICS:
        _parse_macho(data, start, size, "", report, arch_label)
        return
    if size >= len(_AR_MAGIC) and data[start : start + len(_AR_MAGIC)] == _AR_MAGIC:
        _parse_archive(data, start, size, report, arch_label)
        return
    raise MachOParseError(
        "universal slice is neither a Mach-O nor an archive"
        if arch_label
        else "not a Mach-O, universal file or archive"
    )


def _parse_fat(data: bytes, report: FileReport) -> None:
    endian, is_64 = _FAT_MAGICS[data[0:4]]
    (count,) = struct.unpack_from(endian + "I", data, 4)
    entry_size = 32 if is_64 else 20
    _require(
        8 + count * entry_size <= len(data),
        f"universal header claims {count} architectures, which do not fit in the file",
    )
    for index in range(count):
        offset = 8 + index * entry_size
        if is_64:
            cputype, cpusubtype, slice_offset, slice_size = struct.unpack_from(
                endian + "iiQQ", data, offset
            )
        else:
            cputype, cpusubtype, slice_offset, slice_size = struct.unpack_from(
                endian + "iiII", data, offset
            )
        cputype &= 0xFFFFFFFF
        cpusubtype &= 0xFFFFFFFF
        _require(
            slice_offset >= 0 and slice_size >= 0 and slice_offset + slice_size <= len(data),
            "universal header points a slice past the end of the file",
        )
        arch = _arch_name(cputype, cpusubtype)
        # lipo cannot produce a file naming one architecture twice, so a
        # duplicate means the header is not what it claims -- and merging the
        # two would let a slice with no deployment target hide behind the one
        # that has it.
        _require(
            arch not in report.architectures,
            f"universal header names the {arch} architecture twice",
        )
        report.entry_for(arch)
        _parse_container(data, slice_offset, slice_size, report, arch)


def read_report(data: bytes, label: str) -> FileReport | None:
    """Report every deployment target in `data`, or None if it holds no Mach-O.

    None is "this reader does not own this file" -- a header, a plist, a text
    file. Anything that IS a Mach-O container and cannot be read raises.
    """
    report = FileReport(label=label)
    if data[0:4] in _FAT_MAGICS:
        _parse_fat(data, report)
        return report
    if data[0:4] in _MACHO_MAGICS:
        _parse_macho(data, 0, len(data), "", report, None)
        return report
    if data[: len(_AR_MAGIC)] == _AR_MAGIC:
        _parse_archive(data, 0, len(data), report, None)
        return report
    return None


def read_path(path: Path, label: str) -> FileReport | None:
    try:
        data = path.read_bytes()
    except OSError as error:
        raise MachOParseError(f"{label}: cannot be read: {error}") from error
    try:
        return read_report(data, label)
    except MachOParseError as error:
        raise MachOParseError(f"{label}: {error}") from error


def _iter_paths(roots: list[Path]) -> list[Path]:
    found: list[Path] = []
    for root in roots:
        if root.is_dir():
            found.extend(sorted(item for item in root.rglob("*") if item.is_file()))
        else:
            found.append(root)
    return found


def _print_report(report: FileReport) -> None:
    print(f"FILE\t{report.label}\t{report.machos}\t{len(report.records)}")
    for arch in sorted(report.architectures):
        entry = report.architectures[arch]
        print(f"ARCH\t{report.label}\t{arch}\t{entry.machos}\t{len(entry.records)}")
        # Deduplicated: a real archive holds hundreds of members that agree, and
        # a line each would bury the one that does not.
        seen: dict[tuple[str, str], tuple[int, str]] = {}
        for record in entry.records:
            key = (record.platform, record.minos)
            count, example = seen.get(key, (0, record.member or "-"))
            seen[key] = (count + 1, example)
        for (platform, minos), (count, example) in sorted(seen.items()):
            print(f"RECORD\t{report.label}\t{arch}\t{platform}\t{minos}\t{count}\t{example}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Print the deployment target of every Mach-O under the given paths."
    )
    parser.add_argument("paths", nargs="+", type=Path)
    parser.add_argument(
        "--relative-to",
        type=Path,
        default=None,
        help="report paths relative to this directory",
    )
    args = parser.parse_args(argv)

    for path in args.paths:
        if not path.exists():
            print(f"no such path: {path}", file=sys.stderr)
            return 1

    for path in _iter_paths(args.paths):
        label = str(path)
        if args.relative_to is not None:
            try:
                label = str(path.relative_to(args.relative_to))
            except ValueError:
                pass
        try:
            report = read_path(path, label)
        except MachOParseError as error:
            print(error, file=sys.stderr)
            return 1
        if report is not None:
            _print_report(report)
    return 0


if __name__ == "__main__":
    sys.exit(main())
