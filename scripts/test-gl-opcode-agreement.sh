#!/usr/bin/env bash
# Three implementations name the same opcodes, or a frame silently does not draw.
#
# A GL command stream record is twelve bits of opcode and twenty of word count.
# Nothing on either side of the process boundary is typed: the producer writes a
# number and the reader switches on it. So an opcode added to one table and not
# another is not a type error, not a link error, and not a runtime error --
# it is a record the reader rejects, on a device, with the frame not drawing and
# nothing in the log that names the opcode.
#
# THE DRIFT THIS EXISTS TO CATCH has a specific history. There was already a
# JS/Rust agreement test, and it was a HAND-WRITTEN LIST of sixty-nine name and
# value pairs inside the runtime crate. Two things were wrong with it: a list
# someone has to extend is a list that falls behind, and it could only see the
# in-process encoder, because the WebContent producer lives outside that crate
# and a test there would have had to reach across the tree to find it.
#
# So the tables are parsed, not restated, and all three are parsed by the same
# gate:
#
#   engine/crates/frame-wire/src/gl_stream.rs                   (the source)
#   engine/crates/runtime-v8/src/rendering/webgl/00_gl_command_stream.js
#   platforms/apple/WebContent/PerformancePlus/src/gl-opcodes.mjs
#
# Host-only: reads three files.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

python3 - <<'PY'
import re
import sys
from pathlib import Path

SOURCES = {
    "rust": Path("engine/crates/frame-wire/src/gl_stream.rs"),
    "in-process js": Path(
        "engine/crates/runtime-v8/src/rendering/webgl/00_gl_command_stream.js"
    ),
    "webcontent js": Path(
        "platforms/apple/WebContent/PerformancePlus/src/gl-opcodes.mjs"
    ),
}

PATTERNS = {
    "rust": re.compile(r"^pub const (OP_[A-Z0-9_]+): u32 = (\d+);", re.M),
    "in-process js": re.compile(r"^\s*const (OP_[A-Z0-9_]+) = (\d+);", re.M),
    "webcontent js": re.compile(r"^export const (OP_[A-Z0-9_]+) = (\d+);", re.M),
}

HEADER = {
    "rust": re.compile(r"^pub const (MAGIC|STREAM_VERSION): u32 = (0x[0-9A-Fa-f_]+|\d+);", re.M),
    "in-process js": re.compile(r"^\s*const (MAGIC|STREAM_VERSION) = (0x[0-9A-Fa-f]+|\d+);", re.M),
    "webcontent js": re.compile(r"^export const (MAGIC|STREAM_VERSION) = (0x[0-9A-Fa-f]+|\d+);", re.M),
}

problems = []
tables = {}
headers = {}

for name, path in SOURCES.items():
    if not path.is_file():
        problems.append(f"{name}: {path} is missing")
        continue
    text = path.read_text(encoding="utf-8")
    table = {op: int(value) for op, value in PATTERNS[name].findall(text)}
    if not table:
        problems.append(f"{name}: no opcodes parsed out of {path}; the pattern no longer matches")
    tables[name] = table
    headers[name] = {
        key: int(value.replace("_", ""), 0) for key, value in HEADER[name].findall(text)
    }

if problems:
    print("FAIL: the opcode tables could not be read.", file=sys.stderr)
    for problem in problems:
        print(f"  * {problem}", file=sys.stderr)
    raise SystemExit(1)

source = tables["rust"]
print(f"\n  - rust declares {len(source)} opcodes")

for name, table in tables.items():
    if name == "rust":
        continue
    missing = sorted(set(source) - set(table))
    extra = sorted(set(table) - set(source))
    mismatched = sorted(
        (op, source[op], table[op]) for op in set(source) & set(table) if source[op] != table[op]
    )
    if missing:
        problems.append(f"{name} is missing {len(missing)}: {', '.join(missing[:8])}")
    if extra:
        problems.append(f"{name} has {len(extra)} the Rust table does not: {', '.join(extra[:8])}")
    for op, expected, found in mismatched:
        problems.append(f"{name}: {op} is {found}, the Rust table says {expected}")
    if not (missing or extra or mismatched):
        print(f"  - {name} agrees on all {len(table)}")

# The stream header, which is checked before any opcode is read. A producer with
# the right opcodes and the wrong magic emits a stream the reader refuses whole.
for key in ("MAGIC", "STREAM_VERSION"):
    values = {name: header.get(key) for name, header in headers.items()}
    if any(value is None for value in values.values()):
        problems.append(f"{key} is not declared by: {', '.join(n for n, v in values.items() if v is None)}")
        continue
    if len(set(values.values())) != 1:
        problems.append(f"{key} disagrees: {values}")
    else:
        print(f"  - {key} agrees: {hex(values['rust'])}")

# The numbering is two contiguous blocks, and the split is load-bearing:
# fixed-length records are 1..=58 and variable-length uniform records are
# 256..=266, so a reader can tell which shape it is holding from the opcode
# alone. An opcode dropped into the gap would be a fixed-length number the
# variable-length path never checks for, and the record would be read with the
# wrong shape rather than rejected.
FIXED_TOP = 255
values = sorted(source.values())
if len(set(values)) != len(values):
    duplicates = sorted({v for v in values if values.count(v) > 1})
    problems.append(f"opcode numbers are reused: {duplicates}")

fixed = sorted(v for v in values if v <= FIXED_TOP)
variable = sorted(v for v in values if v > FIXED_TOP)
if fixed and fixed != list(range(1, len(fixed) + 1)):
    gaps = [n for n in range(1, fixed[-1] + 1) if n not in set(fixed)]
    problems.append(f"the fixed-length opcodes are not contiguous from 1; missing {gaps[:8]}")
if variable and variable != list(range(256, 256 + len(variable))):
    gaps = [n for n in range(256, variable[-1] + 1) if n not in set(variable)]
    problems.append(f"the variable-length opcodes are not contiguous from 256; missing {gaps[:8]}")
if variable and variable[0] != 256:
    problems.append(f"the variable-length block starts at {variable[0]}, not 256")
print(f"  - {len(fixed)} fixed-length opcodes (1..={fixed[-1] if fixed else 0}), "
      f"{len(variable)} variable-length (256..={variable[-1] if variable else 0})")

print()
if problems:
    print("FAIL: the opcode tables disagree.", file=sys.stderr)
    for problem in problems:
        print(f"  * {problem}", file=sys.stderr)
    print(
        "\n  A record header is twelve bits of opcode and twenty of word count.\n"
        "  Nothing is typed across that boundary: a producer writes a number and\n"
        "  the reader switches on it, so a disagreement here is a frame that does\n"
        "  not draw rather than anything that fails to build.\n",
        file=sys.stderr,
    )
    raise SystemExit(1)

print("PASS: all three opcode tables agree.")
PY
