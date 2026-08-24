#!/usr/bin/env bash
# Report the API surface this build publishes to content.
#
# Runs `tools/api-surface/probe` as ordinary mini-game content through the Linux
# player and captures what it sees. The runtime is the authority here, not the
# sources: reading `99_global_scope*.js` reports what registration *intends*,
# and the two have already disagreed -- `Deno` was once mirrored onto a
# published namespace even though hardening deletes it from globalThis, which
# is how a reachable op table survived. A probe sees the post-snapshot,
# post-hardening truth.
#
# Output: JSON on stdout, or to --out.
#   {"global":[...], "migo":[...], "wx":[...] | null}
#
# Usage:
#   bash scripts/dump-api-surface.sh [--out FILE] [--secs N] [--adapter FILE]
#
# --adapter FILE evaluates an adapter bundle (e.g. migo-wx-adapter's IIFE) ahead
# of the probe, in the same isolate, exactly as a host injects it -- so the `wx`
# names in the output are the ones an adapter actually installed at runtime, not
# the ones its source appears to assign. The engine publishes no `wx` of its own;
# without --adapter the `wx` key is null, which is the honest answer rather than
# an empty list that would read as "the adapter installs nothing".
#
# Note the surface is per **product profile** (api-commerce, api-media, ... are
# cargo features), not per OS: the same profile publishes the same JS names on
# Android and Linux, because the global-scope modules are gated by feature, not
# by target. What Linux does *not* have is device services, so this reports the
# published surface, never whether a call would succeed.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROBE_DIR="$ROOT_DIR/tools/api-surface/probe"
OUT=""
SECS=3
ADAPTER=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --out) OUT="${2:?--out requires a path}"; shift 2 ;;
        --out=*) OUT="${1#*=}"; shift ;;
        --secs) SECS="${2:?--secs requires a number}"; shift 2 ;;
        --secs=*) SECS="${1#*=}"; shift ;;
        --adapter) ADAPTER="${2:?--adapter requires a path}"; shift 2 ;;
        --adapter=*) ADAPTER="${1#*=}"; shift ;;
        -h|--help) sed -n '2,24p' "$0"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

[[ -d "$PROBE_DIR" ]] || { echo "probe bundle missing: $PROBE_DIR" >&2; exit 1; }

raw="$(mktemp)"
STAGE=""
cleanup() { rm -f "$raw"; [[ -n "$STAGE" ]] && rm -rf "$STAGE"; }
trap cleanup EXIT

RUN_DIR="$PROBE_DIR"
if [[ -n "$ADAPTER" ]]; then
    [[ -f "$ADAPTER" ]] || { echo "adapter bundle not found: $ADAPTER" >&2; exit 2; }
    # Concatenated into one entry rather than loaded as a second module, because
    # that is how a host injects an adapter: same isolate, same global, evaluated
    # before any content. Loading it any other way would report a surface no game
    # ever sees.
    STAGE="$(mktemp -d)"
    cp "$PROBE_DIR/game.json" "$STAGE/game.json"
    { cat "$ADAPTER"; echo; cat "$PROBE_DIR/game.js"; } > "$STAGE/game.js"
    RUN_DIR="$STAGE"
    echo "[surface] adapter injected ahead of the probe: $ADAPTER" >&2
fi

if ! bash "$ROOT_DIR/scripts/dev-run-player.sh" "$RUN_DIR" "$SECS" >"$raw" 2>&1; then
    echo "player run failed; last lines:" >&2
    tail -20 "$raw" >&2
    exit 1
fi

surface="$(grep -o '__MIGO_SURFACE__.*' "$raw" | head -1 | sed 's/^__MIGO_SURFACE__//')"

# An empty capture means the probe never ran -- a bundle that fails to evaluate
# still exits the player cleanly, so silence here would otherwise be reported as
# a runtime that publishes nothing.
if [[ -z "$surface" ]]; then
    echo "probe produced no surface marker; the bundle did not evaluate." >&2
    echo "last player output:" >&2
    tail -20 "$raw" >&2
    exit 1
fi

python3 - "$surface" "$([[ -n "$ADAPTER" ]] && echo with-adapter || echo bare)" <<'PY' > "${OUT:-/dev/stdout}"
import json
import sys

data = json.loads(sys.argv[1])
for key in ("global", "migo"):
    if not data.get(key):
        print(f"surface is missing or empty for `{key}`", file=sys.stderr)
        raise SystemExit(1)
# `wx` may legitimately be null (no adapter). An adapter that was asked for and
# installed nothing is a different thing, and must not pass silently.
if len(sys.argv) > 2 and sys.argv[2] == "with-adapter" and not data.get("wx"):
    print("an adapter was injected but no `wx` names appeared; it did not install", file=sys.stderr)
    raise SystemExit(1)
print(json.dumps(data, indent=2, sort_keys=True))
PY

if [[ -n "$OUT" ]]; then
    echo "surface -> $OUT" >&2
fi
