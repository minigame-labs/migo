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
# The marker's own sentence, which says what the stub actually does. A report
# that prints only names has to generalise over all of them, and the general
# sentence is wrong for some: "silently succeeds and does not work" is true of
# `reportEvent` and false of `getPrivacySetting`, which answers truthfully that
# this build has no privacy gate. The specific sentence is already written; it
# was simply being dropped on the floor.
whole_text: dict[str, str] = {}
partial_text: dict[str, dict[str, str]] = {}

for path in sorted(src.rglob("*.js")):
    text = path.read_text(encoding="utf-8", errors="replace")
    # A marker may wrap onto following comment lines, and reading only the first
    # truncated two of them mid-clause in the customer-facing report. A
    # continuation is a comment line that does not itself start a new `@stub` --
    # `14_setting.js` has three markers stacked with no blank line between them,
    # so without that guard the first would swallow the next two.
    markers: list[str] = []
    lines = text.splitlines()
    for index, line in enumerate(lines):
        match = re.search(r"@stub\s+(.*)", line)
        if not match:
            continue
        parts = [match.group(1).strip()]
        for follow in lines[index + 1: index + 4]:
            stripped = follow.strip()
            if not re.match(r"^(//|\*)", stripped):
                break
            if "@stub" in stripped:
                break
            body = re.sub(r"^(//|\*)\s*", "", stripped)
            if not body:
                break
            parts.append(body)
        markers.append(" ".join(parts))

    if not markers:
        continue
    key = path.name
    for marker in markers:
        marker = marker.strip()
        if marker.lower().startswith("all"):
            whole.add(key)
            whole_text.setdefault(key, marker)
            continue
        name = re.match(r"([A-Za-z_$][\w$]*)", marker)
        # A bare "@stub - ..." names nothing; it documents a callback the host
        # invokes rather than a published entry point, so there is nothing to
        # attribute and nothing to report.
        if name:
            partial.setdefault(key, set()).add(name.group(1))
            partial_text.setdefault(key, {}).setdefault(name.group(1), marker)

# 2. What each 99_global_scope file publishes, and from which module.
stubs: set[str] = set()
attributed: dict[str, str] = {}
describes: dict[str, str] = {}
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
        named = partial.get(module, set())
        # A marker may name the *class* a factory returns rather than the factory
        # itself -- `19_log_manager.js` says `@stub RealtimeLogManager` while the
        # published name is `getRealtimeLogManager`. Resolving `X` to a published
        # `getX` from the same module is an explicit rule, not a fuzzy match: the
        # module has to publish it, and the name has to be exactly `get` + the
        # marker.
        #
        # The alternative was to edit the marker. That is a one-line comment in a
        # file under `runtime-v8`, and every `.js` and `.rs` there is inside the
        # V8 snapshot fingerprint -- so correcting a comment costs regenerating
        # eight snapshots on real hardware. A convention that taxes documentation
        # fixes that heavily is a convention people stop obeying.
        if (module in whole
                or member in named
                or published in named
                or any(published == "get" + marker for marker in named)):
            stubs.add(published)
            attributed[published] = module
            # Most specific wins: a marker naming this entry point, then one
            # naming the class it returns, then the module-wide sentence.
            texts = partial_text.get(module, {})
            describes[published] = (
                texts.get(published)
                or texts.get(member)
                or next((texts[m] for m in texts if published == "get" + m), None)
                or whole_text.get(module, "")
            )

# 3. A marker nobody can attribute is a marker that has stopped meaning
#    anything -- a renamed module, a deleted export. Say so rather than
#    silently emitting a shorter list.
orphans = sorted(
    module for module in (whole | set(partial))
    if module not in set(attributed.values())
)

if fmt == "json":
    print(json.dumps(
        {"stubs": sorted(stubs), "by_module": attributed,
         "describes": describes, "orphan_markers": orphans},
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
