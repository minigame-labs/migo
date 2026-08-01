#!/usr/bin/env bash
# Report the API surface this build publishes to content.
#
# Runs `tools/api-surface/probe` as ordinary mini-game content through the Linux
# player and captures what it sees. The runtime is the authority here, not the
# sources: reading `99_global_scope*.js` reports what registration *intends*,
# and the two have already disagreed -- `Deno` was mirrored onto `wx` even
# though hardening deletes it from globalThis, which is how a reachable op table
# survived. A probe sees the post-snapshot, post-hardening truth.
#
# Output: JSON on stdout, or to --out.
#   {"global":[...], "wx":[...], "migo":[...]}
#
# Usage:
#   bash scripts/dump-api-surface.sh [--out FILE] [--secs N]
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

while [[ $# -gt 0 ]]; do
    case "$1" in
        --out) OUT="${2:?--out requires a path}"; shift 2 ;;
        --out=*) OUT="${1#*=}"; shift ;;
        --secs) SECS="${2:?--secs requires a number}"; shift 2 ;;
        --secs=*) SECS="${1#*=}"; shift ;;
        -h|--help) sed -n '2,24p' "$0"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

[[ -d "$PROBE_DIR" ]] || { echo "probe bundle missing: $PROBE_DIR" >&2; exit 1; }

raw="$(mktemp)"
trap 'rm -f "$raw"' EXIT

if ! bash "$ROOT_DIR/scripts/dev-run-player.sh" "$PROBE_DIR" "$SECS" >"$raw" 2>&1; then
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

python3 - "$surface" <<'PY' > "${OUT:-/dev/stdout}"
import json
import sys

data = json.loads(sys.argv[1])
for key in ("global", "wx", "migo"):
    if not data.get(key):
        print(f"surface is missing or empty for `{key}`", file=sys.stderr)
        raise SystemExit(1)
print(json.dumps(data, indent=2, sort_keys=True))
PY

if [[ -n "$OUT" ]]; then
    echo "surface -> $OUT" >&2
fi
