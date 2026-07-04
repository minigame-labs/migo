#!/usr/bin/env bash
# Writes <bin>.manifest.json next to a generated V8 snapshot, capturing the
# fingerprint (js_sources_sha256 + deno_core_version) that
# check-snapshot-freshness.sh compares against.
#
# Shared by gen-snapshot.sh (arm64, on a real device) and build-snapshot.yml
# (x86_64, in CI) so BOTH arches emit byte-identical manifest formats and
# identical fingerprints — that is the whole point of the freshness gate.
#
# Usage: write-snapshot-manifest.sh <arch> <path/to/SNAPSHOT-<arch>.bin>
set -euo pipefail

ARCH="${1:?usage: write-snapshot-manifest.sh <arch> <bin-path>}"
BIN="${2:?usage: write-snapshot-manifest.sh <arch> <bin-path>}"
[[ -s "$BIN" ]] || { echo "ERROR: snapshot missing or empty: $BIN" >&2; exit 1; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"
ENGINE="$ROOT/engine"

# shellcheck source=scripts/lib/snapshot-fingerprint.sh
source "$ROOT/scripts/lib/snapshot-fingerprint.sh"
JS_HASH="$(snapshot_js_hash "$ROOT")"
DENO_CORE_VER="$(snapshot_deno_core_version "$ENGINE")"

MANIFEST="$BIN.manifest.json"
cat > "$MANIFEST" <<EOF
{
  "arch": "$ARCH",
  "snapshot_size": $(stat -c %s "$BIN"),
  "snapshot_sha256": "$(sha256sum "$BIN" | awk '{print $1}')",
  "js_sources_sha256": "$JS_HASH",
  "deno_core_version": "$DENO_CORE_VER",
  "git_commit": "$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)",
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF
echo "manifest -> $MANIFEST"
