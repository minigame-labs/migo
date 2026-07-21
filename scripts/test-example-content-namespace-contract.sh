#!/usr/bin/env bash
# The example content must not call capabilities on a namespace that does not
# carry them.
#
# `97_wx_namespace.js` mirrors most globals onto `wx`, but deliberately keeps a
# few off it: wx has no gamepad API, so those names live on `migo` (and reach
# browser content as `navigator.getGamepads()` through the HTML5 adapter).
# Calling `wx.getGamepads()` therefore throws TypeError on the first frame,
# which aborts paint and leaves the screen black -- with no clue in the failure
# that a namespace was the problem. That is exactly how it shipped: every probe
# written for the gamepad and IME work called `wx.getGamepads()`, and the black
# screen was read as a frame-driving or rendering fault on the device.
#
# The forbidden set is parsed out of the runtime source rather than repeated
# here, so a capability added to `_NON_WX` later is covered without touching
# this file.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT_DIR" <<'PY'
from __future__ import annotations

import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1]).resolve()
errors: list[str] = []

namespace_source = root / "engine/crates/runtime-v8/src/97_wx_namespace.js"
if not namespace_source.is_file():
    print(f"ERROR: wx namespace source not found at {namespace_source}", file=sys.stderr)
    sys.exit(1)

text = namespace_source.read_text(encoding="utf-8")
block = re.search(r"const _NON_WX = new Set\(\[(.*?)\]\);", text, re.S)
if block is None:
    # Without this the gate cannot fail, so it must not pass either.
    print(
        "ERROR: could not find the _NON_WX set in 97_wx_namespace.js; "
        "this gate reads it to know which names are off the wx namespace",
        file=sys.stderr,
    )
    sys.exit(1)

forbidden = re.findall(r'"([^"]+)"', block.group(1))
if not forbidden:
    print("ERROR: the _NON_WX set parsed empty; the gate would pass vacuously", file=sys.stderr)
    sys.exit(1)

examples = root / "examples"
scanned = 0
for source_path in sorted(examples.rglob("*.js")):
    if "/build/" in str(source_path):
        continue
    scanned += 1
    source = source_path.read_text(encoding="utf-8")
    for name in forbidden:
        # Word boundary, not substring: `wx.getGamepadsLater` is a different name.
        if re.search(r"\bwx\." + re.escape(name) + r"\b", source):
            relative = source_path.relative_to(root)
            errors.append(
                f"{relative} calls wx.{name}, but {name} is in _NON_WX and is "
                f"only exposed on `migo` (browser content reaches it through "
                f"the adapter as navigator.{name})"
            )

if scanned == 0:
    print("ERROR: no example JS found to scan; the gate would pass vacuously", file=sys.stderr)
    sys.exit(1)

if errors:
    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    sys.exit(1)

print(f"OK: {scanned} example sources use no wx-namespaced capability that wx does not carry")
PY
