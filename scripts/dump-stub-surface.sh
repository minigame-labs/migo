#!/usr/bin/env bash
# Which published names are stubs?
#
# `scripts/dump-api-surface.sh` answers "what does this build publish". That is
# the question a prescreen report has been answering, and it is half an answer:
# some published names do nothing. `system/17_analytics.js` says it plainly --
# "All functions are no-op stubs that **silently succeed**" -- so content calls
# `reportEvent`, gets no error, and no event is recorded anywhere.
#
# A prescreen that counts those as supported hands a customer "0 gaps" for a
# bundle whose analytics will never report. That is the same failure the scanner
# already refuses elsewhere: a confident answer that is wrong in the direction
# nobody checks.
#
# DERIVED, NOT LISTED. A hand-written table of stub names would be wrong within
# a release -- somebody implements one and the table still calls it a stub, or
# adds one and the table has never heard of it. This reads the sources:
#
#   * a module carrying `@stub` in its header is stubbed wholesale when the
#     marker says "All", and per-name when the marker names names;
#   * `99_global_scope*.js` binds published names to those modules through a
#     namespace import, which is the same file the runtime actually registers
#     from -- so the mapping cannot drift from what content sees.
#
# Output: one name per line, sorted. `--json` for the machine form.
#
# Usage:
#   bash scripts/dump-stub-surface.sh [--json] [--out FILE]
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT_DIR/engine/crates/runtime-v8/src"
FORMAT=text
OUT=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --json) FORMAT=json; shift ;;
        --out) OUT="${2:?--out requires a path}"; shift 2 ;;
        --out=*) OUT="${1#*=}"; shift ;;
        -h|--help) sed -n '2,28p' "$0"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

[[ -d "$SRC" ]] || { echo "runtime-v8 sources not at $SRC" >&2; exit 1; }

python3 - "$SRC" "$FORMAT" <<'PY' > "${OUT:-/dev/stdout}"
from __future__ import annotations

import json
import pathlib
import re
import sys

src = pathlib.Path(sys.argv[1])
fmt = sys.argv[2]

# 1. Which module files declare themselves stubbed, and how completely.
#    "@stub All ..." / "@stub <Name>: ..." / "@stub <name> ..." / bare "@stub -".
whole: set[str] = set()          # module path -> every published name is a stub
partial: dict[str, set[str]] = {}  # module path -> just these names

for path in sorted(src.rglob("*.js")):
    text = path.read_text(encoding="utf-8", errors="replace")
    markers = re.findall(r"@stub\s+(.*)", text)
    if not markers:
        continue
    key = path.name
    for marker in markers:
        marker = marker.strip()
        if marker.lower().startswith("all"):
            whole.add(key)
            continue
        name = re.match(r"([A-Za-z_$][\w$]*)", marker)
        # A bare "@stub - ..." names nothing; it documents a callback the host
        # invokes rather than a published entry point, so there is nothing to
        # attribute and nothing to report.
        if name:
            partial.setdefault(key, set()).add(name.group(1))

# 2. What each 99_global_scope file publishes, and from which module.
stubs: set[str] = set()
attributed: dict[str, str] = {}
for scope in sorted(src.rglob("99_global_scope*.js")):
    text = scope.read_text(encoding="utf-8", errors="replace")
    alias_to_module = {
        alias: module.rsplit("/", 1)[-1]
        for alias, module in re.findall(
            r"import\s+\*\s+as\s+(\w+)\s+from\s+'([^']+)'", text
        )
    }
    for published, alias, member in re.findall(
        r"^\s+([A-Za-z_$][\w$]*)\s*:\s*core\.prop\w*\(\s*(\w+)\.([\w$]+)", text, re.M
    ):
        module = alias_to_module.get(alias)
        if module is None:
            continue
        if module in whole or member in partial.get(module, ()) or published in partial.get(module, ()):
            stubs.add(published)
            attributed[published] = module

# 3. A marker nobody can attribute is a marker that has stopped meaning
#    anything -- a renamed module, a deleted export. Say so rather than
#    silently emitting a shorter list.
orphans = sorted(
    module for module in (whole | set(partial))
    if module not in set(attributed.values())
)

if fmt == "json":
    print(json.dumps(
        {"stubs": sorted(stubs), "by_module": attributed, "orphan_markers": orphans},
        indent=2, sort_keys=True, ensure_ascii=False,
    ))
else:
    for name in sorted(stubs):
        print(name)

if orphans:
    print(
        "warning: @stub markers with no published name: " + ", ".join(orphans),
        file=sys.stderr,
    )
PY

if [[ -n "$OUT" ]]; then
    echo "stub surface -> $OUT" >&2
fi
