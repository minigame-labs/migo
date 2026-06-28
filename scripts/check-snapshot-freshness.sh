#!/usr/bin/env bash
# =============================================================================
# Verify committed V8 snapshots are not stale, WITHOUT a device.
# =============================================================================
#
# A stale snapshot is the main footgun of the snapshot system:
#   * changed extension JS  -> the runtime silently runs the OLD baked JS
#   * bumped deno_core/V8    -> hard crash at load (V8 magic mismatch)
#
# Both failure inputs (extension JS, deno_core version) are PLATFORM-INDEPENDENT,
# so we can detect staleness on any host with no emulator/device: recompute the
# fingerprint from the current tree and compare it to each snapshot's committed
# manifest (written by gen-snapshot.sh).
#
# Intended as a CI gate on the release path and a local pre-build sanity check.
#
# Usage:
#   scripts/check-snapshot-freshness.sh [arch...]      # default: all present
#
# Exit codes: 0 = fresh (or no snapshots present -> from-source, safe),
#             1 = a present snapshot is stale (regenerate with gen-snapshot.sh).
# =============================================================================
set -euo pipefail

c_info() { echo -e "\033[0;36m[INFO] $*\033[0m"; }
c_ok()   { echo -e "\033[0;32m[OK]   $*\033[0m"; }
c_warn() { echo -e "\033[0;33m[WARN] $*\033[0m"; }
c_err()  { echo -e "\033[0;31m[STALE] $*\033[0m" >&2; }

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENGINE="$ROOT/engine"
SNAP_DIR="$ENGINE/crates/js-runtime/snapshots"
# shellcheck source=scripts/lib/snapshot-fingerprint.sh
source "$ROOT/scripts/lib/snapshot-fingerprint.sh"

CUR_JS="$(snapshot_js_hash "$ROOT")"
CUR_DENO="$(snapshot_deno_core_version "$ENGINE")"
c_info "current fingerprint: deno_core=$CUR_DENO js=${CUR_JS:0:12}…"

# Which arches to check.
if [[ $# -gt 0 ]]; then
  ARCHES=("$@")
else
  ARCHES=()
  for f in "$SNAP_DIR"/SNAPSHOT-*.bin; do
    [[ -e "$f" ]] || continue
    a="${f##*/SNAPSHOT-}"; ARCHES+=("${a%.bin}")
  done
fi

if [[ "${#ARCHES[@]}" -eq 0 ]]; then
  c_warn "no snapshots present in $SNAP_DIR — builds use the from-source fallback (safe, slower cold start)."
  exit 0
fi

# json field reader (no jq dependency)
jget() { sed -n "s/.*\"$2\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" "$1" | head -1; }

stale=0
for arch in "${ARCHES[@]}"; do
  snap="$SNAP_DIR/SNAPSHOT-$arch.bin"
  man="$snap.manifest.json"
  [[ -f "$snap" ]] || { c_err "$arch: snapshot file missing ($snap)"; stale=1; continue; }
  [[ -f "$man" ]]  || { c_err "$arch: manifest missing ($man) — regenerate with gen-snapshot.sh"; stale=1; continue; }

  m_js="$(jget "$man" js_sources_sha256)"
  m_deno="$(jget "$man" deno_core_version)"

  if [[ "$m_js" != "$CUR_JS" || "$m_deno" != "$CUR_DENO" ]]; then
    c_err "$arch: STALE."
    [[ "$m_deno" != "$CUR_DENO" ]] && echo "        deno_core: manifest=$m_deno current=$CUR_DENO"
    [[ "$m_js"   != "$CUR_JS"   ]] && echo "        extension JS changed (manifest=${m_js:0:12}… current=${CUR_JS:0:12}…)"
    echo "        -> regenerate: scripts/gen-snapshot.sh $([[ $arch == aarch64 ]] && echo arm64 || echo "$arch")"
    stale=1
  else
    c_ok "$arch: fresh."
  fi
done

[[ "$stale" -eq 0 ]] || exit 1
