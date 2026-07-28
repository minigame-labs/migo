#!/usr/bin/env bash
# ============================================================
# Fetch prebuilt V8 archives for one or more targets and verify them.
# Location: scripts/fetch-v8-archives.sh
#
# These are build products of scripts/build-v8-{android,linux}.sh. They used to
# be tracked in Git LFS, which made every CI checkout spend LFS bandwidth on
# ~247 MB and eventually exhausted the repository's quota -- at which point
# checkout failed and nothing could build, for a reason that had nothing to do
# with the code. Release assets do not count against that quota, so the archives
# live on a release and are pulled here instead.
#
# Integrity does not depend on the transport. Each archive is checked against the
# `hashes.archive` sha256 in the committed component-manifest.json, which the
# release gate already treats as the authority on what a valid archive is. That
# is a stronger statement than LFS made: LFS guaranteed the bytes matched an
# object id, but nothing tied that object to the manifest.
#
# A target with no committed manifest cannot be fetched, by design: without one
# there is nothing to say what a valid archive is, and downloading bytes that
# cannot be checked would make this a plain transport wearing a verification
# comment. That is why x86_64-pc-windows-msvc is absent below -- see
# scripts/build-v8-windows.sh, which deliberately does not emit a manifest until
# a Windows lock file exists to verify one against.
#
# Usage: bash scripts/fetch-v8-archives.sh [--check] [--all] [target...]
#   --check      verify what is already present; download nothing
#   --all        every target known to this script
#   target...    one or more of: aarch64, x86_64, x86_64-linux-gnu
#
# With no target given, the two Android targets are fetched. That is what the
# Android build and CI need, and keeping it the default means adding a target
# here never silently makes those builds download more than they use.
# ============================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
V8_DIR="$ROOT/engine/third_party/rusty_v8"
TAG="${MIGO_V8_ARCHIVE_TAG:-v8-archives-e6a88b3}"
# Overridable so the archives can be served from somewhere else (a mirror, or a
# fork's own release) without editing this script.
BASE_URL="${MIGO_V8_ARCHIVE_BASE_URL:-https://github.com/minigame-labs/migo/releases/download/$TAG}"

# Targets this script knows how to fetch and verify. The directory name doubles
# as the asset suffix, so a release asset is always librusty_v8-<target>.a and
# there is no second naming scheme to keep in sync.
KNOWN_TARGETS=(aarch64 x86_64 x86_64-linux-gnu)
DEFAULT_TARGETS=(aarch64 x86_64)

info() { printf '\033[0;36m[v8-fetch] %s\033[0m\n' "$*"; }
ok()   { printf '\033[0;32m[v8-fetch] %s\033[0m\n' "$*"; }
err()  { printf '\033[0;31m[v8-fetch] %s\033[0m\n' "$*" >&2; }

CHECK_ONLY=0
requested=()
for arg in "$@"; do
    case "$arg" in
        --check) CHECK_ONLY=1 ;;
        --all)   requested+=("${KNOWN_TARGETS[@]}") ;;
        -*)
            err "unknown option: $arg"
            err "usage: fetch-v8-archives.sh [--check] [--all] [target...]"
            exit 2
            ;;
        *)
            match=0
            for known in "${KNOWN_TARGETS[@]}"; do
                [[ "$arg" == "$known" ]] && match=1 && break
            done
            if (( match == 0 )); then
                err "unknown target: $arg"
                err "known targets: ${KNOWN_TARGETS[*]}"
                exit 2
            fi
            requested+=("$arg")
            ;;
    esac
done
(( ${#requested[@]} == 0 )) && requested=("${DEFAULT_TARGETS[@]}")

# De-duplicate while preserving order, so `--all x86_64` does not fetch twice.
targets=()
for t in "${requested[@]}"; do
    seen=0
    for u in ${targets[@]+"${targets[@]}"}; do
        [[ "$t" == "$u" ]] && seen=1 && break
    done
    (( seen == 0 )) && targets+=("$t")
done

expected_sha() {
    python3 -c "
import json, sys
with open(sys.argv[1]) as f:
    print(json.load(f).get('hashes', {}).get('archive', ''))
" "$1"
}

failures=0
for target in "${targets[@]}"; do
    manifest="$V8_DIR/$target/component-manifest.json"
    archive="$V8_DIR/$target/librusty_v8.a"
    asset="librusty_v8-$target.a"

    if [[ ! -f "$manifest" ]]; then
        err "no component manifest for $target -- cannot say what a valid archive is"
        failures=$((failures + 1)); continue
    fi
    want="$(expected_sha "$manifest")"
    if [[ -z "$want" ]]; then
        err "$target manifest records no archive hash"
        failures=$((failures + 1)); continue
    fi

    if [[ -f "$archive" ]]; then
        have="$(sha256sum "$archive" | cut -d' ' -f1)"
        if [[ "$have" == "$want" ]]; then
            ok "$target: already present and matches the manifest"
            continue
        fi
        # A wrong archive is worse than a missing one: it links and produces a
        # binary whose provenance chain is a lie. Replace it, never keep it.
        err "$target: present but does NOT match the manifest -- refetching"
        rm -f "$archive"
    fi

    if [[ "$CHECK_ONLY" == "1" ]]; then
        err "$target: missing or mismatched (checking only, nothing downloaded)"
        failures=$((failures + 1)); continue
    fi

    mkdir -p "$(dirname "$archive")"
    info "$target: downloading $asset"
    if ! curl -fsSL --retry 3 --retry-delay 2 -o "$archive" "$BASE_URL/$asset"; then
        err "$target: download failed from $BASE_URL/$asset"
        rm -f "$archive"
        failures=$((failures + 1)); continue
    fi

    have="$(sha256sum "$archive" | cut -d' ' -f1)"
    if [[ "$have" != "$want" ]]; then
        err "$target: sha256 mismatch after download"
        err "  expected $want"
        err "  got      $have"
        # Leaving it on disk would let a later step pick it up.
        rm -f "$archive"
        failures=$((failures + 1)); continue
    fi
    ok "$target: downloaded and verified against the manifest"
done

if (( failures > 0 )); then
    err "$failures archive(s) unavailable or unverified"
    exit 1
fi
ok "${#targets[@]} V8 archive(s) verified: ${targets[*]}"
