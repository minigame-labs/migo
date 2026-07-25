#!/usr/bin/env bash
# An input listener published to content must have native code that can fire it.
#
# The engine's input modules expose listener groups on the `migo` namespace and a
# matching `_internalTrigger*` entry point that native code calls to deliver an
# event. When the trigger has no caller anywhere in the engine, the listener is
# still advertised and still accepts a callback -- it simply never runs. Content
# registering it sees nothing happen, on every platform, with no error: the same
# failure shape as the `wx.getGamepads()` bug, where a black
# screen was read for days as a rendering fault.
#
# Both producer channels count, and checking only the first misreports the second:
#
#   1. the bridge table in `js_bindings.rs`, resolved by name at isolate init;
#   2. an eval'd JS snippet built in Rust, which is how `_internalTriggerOnShow`
#      is fired (`core/src/runtime/host.rs`) -- it appears in no bridge table.
#
# So a producer is the trigger name occurring in any engine Rust source. Both the
# published set and the producer set are read from the sources rather than listed
# here, so a trigger added later is covered without touching this file.
#
# Scope is the input modules. A dead trigger elsewhere can have a different and
# legitimate cause -- `_internalTriggerAddToFavorites` is a wx API that is declared
# because wx declares it and is waiting on host support -- and deleting those to
# satisfy a gate would shrink wx compatibility rather than fix anything.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT_DIR" <<'PY'
from __future__ import annotations

import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1]).resolve()

input_dir = root / "engine/crates/runtime-v8/src/input"
namespace_source = root / "engine/crates/runtime-v8/src/98_global_scope_window.js"
crates_dir = root / "engine/crates"

for required in (input_dir, namespace_source, crates_dir):
    if not required.exists():
        print(f"ERROR: {required} not found; this gate cannot check anything", file=sys.stderr)
        sys.exit(1)

TRIGGER = re.compile(r"_internalTrigger[A-Za-z0-9_]*")

# Defined: the input modules own these entry points.
defined: dict[str, pathlib.Path] = {}
for source_path in sorted(input_dir.glob("*.js")):
    source = source_path.read_text(encoding="utf-8")
    for match in re.finditer(r"function\s+(_internalTrigger[A-Za-z0-9_]*)\s*\(", source):
        defined[match.group(1)] = source_path

if not defined:
    print(
        "ERROR: no _internalTrigger* function is defined under "
        f"{input_dir.relative_to(root)}; the gate would pass vacuously",
        file=sys.stderr,
    )
    sys.exit(1)

# Published: reachable by content through the `migo` namespace.
namespace_text = namespace_source.read_text(encoding="utf-8")
published_names = set(TRIGGER.findall(namespace_text))
if not published_names:
    print(
        f"ERROR: no _internalTrigger* is published by {namespace_source.name}; "
        "the gate would pass vacuously",
        file=sys.stderr,
    )
    sys.exit(1)

published_input = {name: path for name, path in defined.items() if name in published_names}
if not published_input:
    print(
        "ERROR: no input trigger is published on the namespace; either the input "
        "modules moved or the namespace file changed shape, and the gate would "
        "pass vacuously",
        file=sys.stderr,
    )
    sys.exit(1)

# Producers: either channel, anywhere in the engine's Rust sources.
producers: set[str] = set()
rust_sources = 0
for source_path in crates_dir.rglob("*.rs"):
    if "/target/" in str(source_path):
        continue
    rust_sources += 1
    producers.update(TRIGGER.findall(source_path.read_text(encoding="utf-8", errors="replace")))

if rust_sources == 0 or not producers:
    print(
        "ERROR: scanned no Rust producer; every trigger would look dead and the "
        "gate would fail for the wrong reason",
        file=sys.stderr,
    )
    sys.exit(1)

errors = []
for name, source_path in sorted(published_input.items()):
    if name not in producers:
        errors.append(
            f"{source_path.relative_to(root)} defines {name} and it is published on "
            f"the `migo` namespace, but no engine Rust source ever calls it: content "
            f"can register the listener and it can never fire"
        )

if errors:
    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    print(
        f"Input trigger producer contract: FAIL ({len(errors)} of "
        f"{len(published_input)} published input triggers have no producer)",
        file=sys.stderr,
    )
    sys.exit(1)

print(
    f"Input trigger producer contract: PASS "
    f"({len(published_input)} published input triggers, all with a native producer, "
    f"across {rust_sources} Rust sources)"
)
PY
