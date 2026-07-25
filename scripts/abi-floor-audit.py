#!/usr/bin/env python3
"""Audit an ELF artifact against migo's Linux ABI contract.

The loader floor is a claim about what migo's own artifacts require, so it is
checked on linked output rather than on source. The defaults below -- GLIBC
2.31 and GLIBCXX 3.4.28 -- are this repository's authority for that floor:
they admit any distribution no older than Ubuntu 20.04 / Debian 11, which is
what the first Linux release commits to. Host builds are pinned to Chromium's
bullseye sysroot and land comfortably inside it. Parsing is kept separate from
process invocation so it can be unit tested without any binary on disk.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys

# objdump -T prints undefined dynamic symbols as:
#   <addr> DF *UND* <size> (GLIBC_2.17) clock_gettime
# The parentheses are what binutils emits for an undefined versioned symbol;
# some builds and inputs omit them, so both forms are accepted. Unversioned
# entries carry no tag and defined symbols carry "Base"; both must be ignored,
# which is why the tag is required to end in an underscore-version.
_VERSION_RE = re.compile(
    r"\*UND\*\s+\S+\s+\(?(?P<tag>[A-Za-z_]+)_(?P<version>\d[\d.]*)\)?\s+(?P<symbol>\S+)"
)
_DYNAMIC_TABLE_MARKER = "DYNAMIC SYMBOL TABLE:"


class AuditParseError(RuntimeError):
    """The audited output could not be understood.

    Raised rather than returning an empty result, because an empty result is
    indistinguishable from a clean artifact and would turn a parser regression
    into a silent pass.
    """


def check_parse_sanity(text: str, needs: dict[str, set[tuple[int, ...]]]) -> None:
    if _DYNAMIC_TABLE_MARKER in text and not needs:
        raise AuditParseError(
            "objdump reported a dynamic symbol table but no versioned symbol was "
            "parsed from it; the output format is not understood, so the floor "
            "cannot be verified"
        )
# A genuine export is defined in a real section. Symbols parked in *ABS* are
# imports or linker artifacts -- `nm --defined-only` calls them defined, which
# is why this reads objdump instead: nm reported 190 exports for a library
# whose documented surface is 12.
_EXPORT_RE = re.compile(
    r"^[0-9a-fA-F]+\s+(?P<flags>[gw! ][^\s]*(?:\s+[^\s]+)?)\s+"
    r"(?P<section>\S+)\s+[0-9a-fA-F]+\s+"
    r"(?:\([^)]*\)|\S+)\s+(?P<symbol>\S+)\s*$"
)
_NEEDED_RE = re.compile(r"^\s*NEEDED\s+(?P<name>\S+)\s*$")


def _to_tuple(version: str) -> tuple[int, ...]:
    return tuple(int(part) for part in version.split("."))


def format_version(version: tuple[int, ...]) -> str:
    return ".".join(str(part) for part in version)


def parse_version_needs(text: str) -> dict[str, set[tuple[int, ...]]]:
    needs: dict[str, set[tuple[int, ...]]] = {}
    for match in _VERSION_RE.finditer(text):
        needs.setdefault(match["tag"], set()).add(_to_tuple(match["version"]))
    return needs


def max_version(needs: dict[str, set[tuple[int, ...]]], tag: str) -> tuple[int, ...] | None:
    versions = needs.get(tag)
    return max(versions) if versions else None


def offending_symbols(
    text: str, tag: str, ceiling: tuple[int, ...]
) -> list[tuple[str, str]]:
    """Every (versioned-tag, symbol) pair above the ceiling, sorted for stability."""
    found = set()
    for match in _VERSION_RE.finditer(text):
        if match["tag"] != tag:
            continue
        if _to_tuple(match["version"]) > ceiling:
            found.add((f"{tag}_{match['version']}", match["symbol"]))
    return sorted(found)


def parse_exported_symbols(text: str) -> set[str]:
    """Dynamic symbols this object actually defines, from `objdump -T` output."""
    symbols = set()
    for line in text.splitlines():
        if "*UND*" in line or "*ABS*" in line:
            continue
        match = _EXPORT_RE.match(line.rstrip())
        if match and match["section"].startswith("."):
            symbols.add(match["symbol"])
    return symbols


def parse_needed(text: str) -> list[str]:
    return [m["name"] for line in text.splitlines() if (m := _NEEDED_RE.match(line))]


def _run(argv: list[str]) -> str:
    result = subprocess.run(argv, capture_output=True, text=True)
    if result.returncode != 0:
        sys.exit(f"{argv[0]} failed: {result.stderr.strip()}")
    return result.stdout


def _cmd_floor(args: argparse.Namespace) -> int:
    text = _run(["objdump", "-T", args.binary])
    needs = parse_version_needs(text)
    try:
        check_parse_sanity(text, needs)
    except AuditParseError as error:
        print(f"FAIL: {args.binary}: {error}")
        return 1
    status = 0
    for tag, ceiling_str in (("GLIBC", args.max_glibc), ("GLIBCXX", args.max_glibcxx)):
        ceiling = _to_tuple(ceiling_str)
        highest = max_version(needs, tag)
        if highest is None:
            print(f"{tag}: none required")
            continue
        print(
            f"{tag}: requires up to {tag}_{format_version(highest)} "
            f"(floor {tag}_{ceiling_str})"
        )
        if highest > ceiling:
            status = 1
            for versioned, symbol in offending_symbols(text, tag, ceiling):
                print(f"  ABOVE FLOOR: {versioned} {symbol}")
    if status:
        print(f"FAIL: {args.binary} requires symbols above the ABI floor")
    else:
        print(f"OK: {args.binary} is within the ABI floor")
    return status


def _cmd_exports(args: argparse.Namespace) -> int:
    text = _run(["objdump", "-T", args.binary])
    for symbol in sorted(parse_exported_symbols(text)):
        print(symbol)
    return 0


def _cmd_needed(args: argparse.Namespace) -> int:
    for name in parse_needed(_run(["objdump", "-p", args.binary])):
        print(name)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    floor = sub.add_parser("floor", help="check symbol-version floor")
    floor.add_argument("binary")
    floor.add_argument("--max-glibc", default="2.31")
    floor.add_argument("--max-glibcxx", default="3.4.28")
    floor.set_defaults(func=_cmd_floor)

    exports = sub.add_parser("exports", help="list defined dynamic symbols")
    exports.add_argument("binary")
    exports.set_defaults(func=_cmd_exports)

    needed = sub.add_parser("needed", help="list DT_NEEDED entries")
    needed.add_argument("binary")
    needed.set_defaults(func=_cmd_needed)

    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
