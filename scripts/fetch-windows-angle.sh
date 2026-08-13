#!/usr/bin/env bash
# Fetch the pinned ANGLE + d3dcompiler_47 runtime DLLs Windows builds ship,
# verified against contracts/artifact-manifest/windows-angle.lock.json.
#
# ANGLE publishes no official prebuilt Windows binaries (see the lock file's
# "why_self_hosted" note), so these three files are hosted on the same release
# tag the V8 archives already use, for the identical reason: no upstream
# single-file distribution exists to pin a URL against. Same shape as
# fetch-v8-archives.sh -- a committed sha256 checked before the bytes are
# trusted, download failing closed rather than silently.
#
# Usage:
#   scripts/fetch-windows-angle.sh [--check] [DEST_DIR]
#   --check     verify what is already present; download nothing
#   DEST_DIR    default: engine/third_party/angle-windows
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=scripts/lib/python-cmd.sh
source "$SCRIPT_DIR/lib/python-cmd.sh"
LOCK="$ROOT/contracts/artifact-manifest/windows-angle.lock.json"

info() { printf '\033[0;36m[win-angle] %s\033[0m\n' "$*"; }
ok()   { printf '\033[0;32m[win-angle] %s\033[0m\n' "$*"; }
err()  { printf '\033[0;31m[win-angle] %s\033[0m\n' "$*" >&2; }

[[ -f "$LOCK" ]] || { err "pin not found: $LOCK"; exit 1; }

CHECK_ONLY=0
DEST=""
for arg in "$@"; do
    case "$arg" in
        --check) CHECK_ONLY=1 ;;
        -*) err "unknown option: $arg"; exit 2 ;;
        *) DEST="$arg" ;;
    esac
done
DEST="${DEST:-$ROOT/engine/third_party/angle-windows}"

# $LOCK is passed as sys.argv[1], not interpolated into the python source
# string: MSYS/Git-Bash's automatic POSIX-to-DOS path conversion (relied on
# here -- see build-windows-sdk-native.sh for where that conversion is
# deliberately disabled instead, and why) rewrites whole argv tokens handed to
# a native, non-MSYS python.exe, not substrings embedded inside a larger
# string argument. A path baked into the -c source itself reaches python
# unconverted and native Windows has no notion of a `/d/a/...` path.
BASE_URL="$("$(python_cmd)" -c "
import json, sys
with open(sys.argv[1]) as f:
    print(json.load(f)['release'])
" "$LOCK")"
FILES=(libEGL.dll libGLESv2.dll d3dcompiler_47.dll)

expected_sha() {
    "$(python_cmd)" -c "
import json, sys
with open(sys.argv[1]) as f:
    print(json.load(f)['files'][sys.argv[2]]['sha256'])
" "$LOCK" "$1"
}

failures=0
for name in "${FILES[@]}"; do
    path="$DEST/$name"
    want="$(expected_sha "$name")"

    if [[ -f "$path" ]]; then
        have="$(sha256sum "$path" | cut -d' ' -f1)"
        if [[ "$have" == "$want" ]]; then
            ok "$name: already present and matches the pin"
            continue
        fi
        err "$name: present but does NOT match the pin -- refetching"
        rm -f "$path"
    fi

    if [[ "$CHECK_ONLY" == "1" ]]; then
        err "$name: missing or mismatched (checking only, nothing downloaded)"
        failures=$((failures + 1)); continue
    fi

    mkdir -p "$DEST"
    info "downloading $name"
    if ! curl -fsSL --retry 3 --retry-delay 2 -o "$path" "$BASE_URL/$name"; then
        err "$name: download failed from $BASE_URL/$name"
        rm -f "$path"
        failures=$((failures + 1)); continue
    fi

    have="$(sha256sum "$path" | cut -d' ' -f1)"
    if [[ "$have" != "$want" ]]; then
        err "$name: sha256 mismatch after download"
        err "  expected $want"
        err "  got      $have"
        rm -f "$path"
        failures=$((failures + 1)); continue
    fi
    ok "$name: downloaded and verified against the pin"
done

if (( failures > 0 )); then
    err "$failures file(s) unavailable or unverified"
    exit 1
fi
ok "ANGLE runtime verified in $DEST"
