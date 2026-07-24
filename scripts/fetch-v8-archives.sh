#!/usr/bin/env bash
# ============================================================
# Fetch the prebuilt Android V8 archives and verify them.
# Location: scripts/fetch-v8-archives.sh
#
# These are build products of scripts/build-v8-android.sh. They used to be
# tracked in Git LFS, which made every CI checkout spend LFS bandwidth on ~247 MB
# and eventually exhausted the repository's quota -- at which point checkout
# failed and nothing could build, for a reason that had nothing to do with the
# code. Release assets do not count against that quota, so the archives live on a
# release and are pulled here instead.
#
# Integrity does not depend on the transport. Each archive is checked against the
# `hashes.archive` sha256 in the committed component-manifest.json, which the
# release gate already treats as the authority on what a valid archive is. That
# is a stronger statement than LFS made: LFS guaranteed the bytes matched an
# object id, but nothing tied that object to the manifest.
#
# Usage: bash scripts/fetch-v8-archives.sh [--check]
#   --check   verify what is already present; download nothing
# ============================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
V8_DIR="$ROOT/engine/third_party/rusty_v8"
TAG="${MIGO_V8_ARCHIVE_TAG:-v8-archives-e6a88b3}"
# Overridable so the archives can be served from somewhere else (a mirror, or a
# fork's own release) without editing this script.
BASE_URL="${MIGO_V8_ARCHIVE_BASE_URL:-https://github.com/minigame-labs/migo/releases/download/$TAG}"

CHECK_ONLY=0
[[ "${1:-}" == "--check" ]] && CHECK_ONLY=1

info() { printf '\033[0;36m[v8-fetch] %s\033[0m\n' "$*"; }
ok()   { printf '\033[0;32m[v8-fetch] %s\033[0m\n' "$*"; }
err()  { printf '\033[0;31m[v8-fetch] %s\033[0m\n' "$*" >&2; }

expected_sha() {
    python3 -c "
import json, sys
with open(sys.argv[1]) as f:
    print(json.load(f).get('hashes', {}).get('archive', ''))
" "$1"
}

failures=0
for abi in aarch64 x86_64; do
    manifest="$V8_DIR/$abi/component-manifest.json"
    archive="$V8_DIR/$abi/librusty_v8.a"
    asset="librusty_v8-$abi.a"

    if [[ ! -f "$manifest" ]]; then
        err "no component manifest for $abi -- cannot say what a valid archive is"
        failures=$((failures + 1)); continue
    fi
    want="$(expected_sha "$manifest")"
    if [[ -z "$want" ]]; then
        err "$abi manifest records no archive hash"
        failures=$((failures + 1)); continue
    fi

    if [[ -f "$archive" ]]; then
        have="$(sha256sum "$archive" | cut -d' ' -f1)"
        if [[ "$have" == "$want" ]]; then
            ok "$abi: already present and matches the manifest"
            continue
        fi
        # A wrong archive is worse than a missing one: it links and produces a
        # binary whose provenance chain is a lie. Replace it, never keep it.
        err "$abi: present but does NOT match the manifest -- refetching"
        rm -f "$archive"
    fi

    if [[ "$CHECK_ONLY" == "1" ]]; then
        err "$abi: missing or mismatched (checking only, nothing downloaded)"
        failures=$((failures + 1)); continue
    fi

    mkdir -p "$(dirname "$archive")"
    info "$abi: downloading $asset"
    if ! curl -fsSL --retry 3 --retry-delay 2 -o "$archive" "$BASE_URL/$asset"; then
        err "$abi: download failed from $BASE_URL/$asset"
        rm -f "$archive"
        failures=$((failures + 1)); continue
    fi

    have="$(sha256sum "$archive" | cut -d' ' -f1)"
    if [[ "$have" != "$want" ]]; then
        err "$abi: sha256 mismatch after download"
        err "  expected $want"
        err "  got      $have"
        # Leaving it on disk would let a later step pick it up.
        rm -f "$archive"
        failures=$((failures + 1)); continue
    fi
    ok "$abi: downloaded and verified against the manifest"
done

if (( failures > 0 )); then
    err "$failures archive(s) unavailable or unverified"
    exit 1
fi
ok "both Android V8 archives verified"
