#!/usr/bin/env bash
# Every `@stub` marker must resolve to a name this build publishes.
#
# The marker is the only record that a published name does nothing. A prescreen
# report reads the derived list and tells a customer which of the APIs their
# bundle calls are inert -- `system/17_analytics.js` says of its own functions
# that they are "no-op stubs that **silently succeed**", so a bundle depending on
# them looks correct in every count and still does not work.
#
# A marker that names something no longer published resolves to nothing, and the
# derivation silently returns a shorter list. Nothing is red; the customer-facing
# report just stops mentioning a stub. That is the failure this gate exists for,
# and it is not hypothetical: `19_log_manager.js` carried `@stub
# RealtimeLogManager` -- the *class*, while the published entry point is
# `getRealtimeLogManager` -- so the very first run of the derivation dropped it.
#
# It also refuses the opposite: markers present but nothing derived at all, which
# is what a renamed registration file or a changed binding shape would look like.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/.."
ROOT_DIR="$(cd "$ROOT_DIR" && pwd)"

fail() { echo "FAIL: $*" >&2; exit 1; }

# Via a file, and via a *quoted* heredoc. This used to interpolate the JSON into
# an unquoted heredoc as `json.loads('''$json''')`, so the shell pasted it into a
# Python string literal and Python then unescaped it a second time. The first
# stub description containing a `"` -- `18_crypto.js` says its methods fail with
# "not supported" -- turned the JSON into a syntax error, and the gate died on a
# traceback instead of reporting anything about stubs.
json_file="$(mktemp)"
trap 'rm -f "$json_file"' EXIT
bash "$ROOT_DIR/scripts/dump-stub-surface.sh" --json > "$json_file" 2>/dev/null \
    || fail "scripts/dump-stub-surface.sh could not run"

python3 - "$ROOT_DIR" "$json_file" <<'PY'
import json, pathlib, sys, re

data = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
root = pathlib.Path(sys.argv[1])

orphans = data.get("orphan_markers") or []
stubs = data.get("stubs") or []

if orphans:
    print("FAIL: these files carry an @stub marker that resolves to no published name:", file=sys.stderr)
    for name in orphans:
        print(f"      {name}", file=sys.stderr)
    print("""
      A marker naming a type, a helper, or a since-renamed export is a marker the
      derivation cannot attribute, so the name silently drops out of the stub list
      and the prescreen report stops warning about it. Name the *published* entry
      point -- the name content writes.""", file=sys.stderr)
    raise SystemExit(1)

# Markers exist somewhere in the tree; the derivation must find something.
src = root / "engine/crates/runtime-v8/src"
marked = [p for p in src.rglob("*.js") if "@stub" in p.read_text(encoding="utf-8", errors="replace")]
if marked and not stubs:
    print(
        f"FAIL: {len(marked)} file(s) carry @stub markers and the derivation produced "
        "an empty list.\n"
        "      The binding shape in 99_global_scope*.js has probably changed, which "
        "means every stub is now reported as a working API.",
        file=sys.stderr,
    )
    raise SystemExit(1)

print(f"PASS: stub surface contract ({len(stubs)} published stub(s), {len(marked)} marked file(s), no orphan markers)")
PY
