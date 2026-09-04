#!/usr/bin/env bash
# The two lists of toggleable WebGL capabilities are one list, kept in two
# languages, and nothing else notices when they stop agreeing.
#
# `isEnabled` is answered on the producer side now: 02_webgl_context.js keeps
# the capabilities content has enabled in one integer per context, so the query
# never leaves the JavaScript thread. That shadow is only as complete as its
# table. A capability missing from the JavaScript list is not a crash and not a
# warning -- `_CAP_BIT.get()` returns `undefined`, `enable()` silently declines
# to record it, and `isEnabled()` answers `false` forever for a capability the
# driver has genuinely turned on. Content that reads its own state back gets a
# lie, on one capability, with no error anywhere.
#
# THE DRIFT THIS EXISTS TO CATCH is the ordinary one: someone adds a capability
# to the renderer's `TOGGLEABLE_CAPS` (which is where a new GL enum naturally
# lands, because that is where the de-duplication lives) and does not know there
# is a second copy in JavaScript. The renderer's list is the authority because
# it is taken from `glow` rather than transcribed; this gate makes the
# JavaScript one follow it.
#
# It also checks the third file: every name the JavaScript list uses has to
# exist in `01_constants.js`, because `WebglConstants.NOT_A_REAL_NAME` is
# `undefined` and `_CAP_BIT` would then map `undefined` to a bit -- a table that
# looks full and has a hole in it. That is not hypothetical: this gate was
# written alongside the shadow, and `RASTERIZER_DISCARD` was missing from
# `01_constants.js` at the time, so `gl.RASTERIZER_DISCARD` was `undefined` and
# WebGL 2 content calling `gl.enable(gl.RASTERIZER_DISCARD)` was passing
# `undefined` to the driver.
#
# Host-only: reads three files, runs nothing.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

python3 - <<'PY'
import re
import sys
from pathlib import Path

RUST = Path("engine/crates/graphics/src/canvas/manager/types.rs")
JS = Path("engine/crates/runtime-v8/src/rendering/webgl/02_webgl_context.js")
CONSTANTS = Path("engine/crates/runtime-v8/src/rendering/webgl/01_constants.js")

problems = []


def block(path: Path, opening: str, closing: str) -> str:
    text = path.read_text()
    start = text.find(opening)
    if start < 0:
        sys.exit(f"ERROR: {path} no longer contains `{opening}`; this gate cannot read it")
    end = text.find(closing, start)
    if end < 0:
        sys.exit(f"ERROR: {path}: `{opening}` is not closed by `{closing}`")
    return text[start + len(opening) : end]


# The renderer's list is the authority: its entries are `glow::NAME`, so the
# numeric values cannot drift from the ones the GL call actually uses.
rust_names = re.findall(r"glow::([A-Z0-9_]+)", block(RUST, "const TOGGLEABLE_CAPS", "];"))

# The JavaScript list names the same capabilities through `WebglConstants`,
# which is the table content itself sees.
js_names = re.findall(
    r"WebglConstants\.([A-Z0-9_]+)", block(JS, "const _TOGGLEABLE_CAPS = [", "];")
)

if not rust_names:
    sys.exit(f"ERROR: parsed no capabilities out of {RUST}; the pattern no longer matches")
if not js_names:
    sys.exit(f"ERROR: parsed no capabilities out of {JS}; the pattern no longer matches")

# Order as well as membership. The two bitmasks never cross the boundary -- each
# side numbers its own bits -- so a reordering is not itself a bug. It is
# checked because the lists are meant to be one list read twice, keeping them
# aligned costs nothing, and a diff that lines up is the difference between
# seeing a missing entry and hunting for it.
if rust_names != js_names:
    only_rust = [n for n in rust_names if n not in js_names]
    only_js = [n for n in js_names if n not in rust_names]
    if only_rust:
        problems.append(
            f"in the renderer's TOGGLEABLE_CAPS and not in the JavaScript "
            f"_TOGGLEABLE_CAPS: {', '.join(only_rust)}"
        )
    if only_js:
        problems.append(
            f"in the JavaScript _TOGGLEABLE_CAPS and not in the renderer's "
            f"TOGGLEABLE_CAPS: {', '.join(only_js)}"
        )
    if not only_rust and not only_js:
        problems.append(
            "the two lists hold the same capabilities in different orders:\n"
            f"    rust:       {', '.join(rust_names)}\n"
            f"    javascript: {', '.join(js_names)}"
        )

# Every name the JavaScript list uses must resolve. `WebglConstants.MISSING` is
# `undefined`, which becomes a bit in `_CAP_BIT` keyed on `undefined` -- a
# ten-entry table with nine capabilities in it.
declared = set(re.findall(r"^\s{4}([A-Z0-9_]+):\s*\d+,", CONSTANTS.read_text(), re.M))
undeclared = [n for n in js_names if n not in declared]
if undeclared:
    problems.append(
        f"used as WebglConstants.<name> but not declared in {CONSTANTS.name}, so it is "
        f"`undefined` at runtime: {', '.join(undeclared)}"
    )

# GL ES initial state: DITHER is the one capability that starts enabled. The
# shadow seeds itself from this, and seeding it wrong is invisible until content
# reads back a capability it never touched.
initial = re.search(r"const _CAP_INITIAL = _CAP_BIT\.get\(WebglConstants\.([A-Z0-9_]+)\)", JS.read_text())
if initial is None:
    problems.append(
        "cannot find `const _CAP_INITIAL = _CAP_BIT.get(WebglConstants.<name>)`; the "
        "shadow's initial state is no longer stated in a form this gate can check"
    )
elif initial.group(1) != "DITHER":
    problems.append(
        f"the capability shadow starts with {initial.group(1)} enabled; GL ES starts "
        f"with DITHER enabled and every other capability disabled"
    )

if problems:
    print("FAIL: the WebGL capability tables disagree", file=sys.stderr)
    for p in problems:
        print(f"  - {p}", file=sys.stderr)
    sys.exit(1)

print(f"  {len(rust_names)} toggleable capabilities, same names and same order in both tables")
print(f"  all {len(js_names)} declared in 01_constants.js")
print("  the shadow starts with DITHER enabled, as GL ES does")
print()
print("PASS: the producer-side capability shadow covers the renderer's capability set.")
PY
