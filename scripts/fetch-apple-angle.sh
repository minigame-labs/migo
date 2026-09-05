#!/usr/bin/env bash
# Fetch the pinned ANGLE-Metal libraries Apple builds ship, verified against
# contracts/artifact-manifest/apple-angle.lock.json.
#
# ANGLE publishes no official prebuilt binaries for any platform -- the finding
# windows-angle.lock.json's "why_self_hosted" note records, checked again for
# Apple -- so these are hosted on a release tag of this repository, for the
# identical reason the V8 archives are: no upstream single-file distribution
# exists to pin a URL against. Same shape as fetch-windows-angle.sh and
# fetch-v8-archives.sh: a committed sha256 checked before the bytes are trusted,
# and a download that fails closed rather than silently.
#
# WHY THE ENGINE NEEDS THESE. There is no GL framework on iOS. rustc's own link
# line for the Apple slices says so -- macOS answers `-framework OpenGL` and iOS
# answers nothing -- and Skia is configured for its GL backend. ANGLE over Metal
# fills that gap.
#
# WHY ONE ARCHIVE PER PLATFORM rather than one asset per library, which is what
# the Windows pin does: ANGLE's own `angle_shared_library` template switches to
# `ios_framework_bundle` when `is_ios`, so an iOS product is a DIRECTORY. A
# directory is not a release asset.
#
# Usage:
#   scripts/fetch-apple-angle.sh [<platform> ...] [--check] [--dest DIR]
#   <platform>  ios, ios-simulator or macos; default is every platform pinned
#   --check     verify what is already present; download nothing
#   --dest DIR  where to unpack (default engine/third_party/, which is where
#               scripts/build-angle-apple.sh installs its own build and where
#               the xcframework assembly reads from)
#
# bash 3.2: this runs on macOS, where /bin/bash is 3.2. See
# scripts/test-macos-bash32-contract.sh.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=scripts/lib/python-cmd.sh
source "$SCRIPT_DIR/lib/python-cmd.sh"
LOCK="$ROOT/contracts/artifact-manifest/apple-angle.lock.json"

info() { printf '\033[0;36m[apple-angle] %s\033[0m\n' "$*"; }
ok()   { printf '\033[0;32m[apple-angle] %s\033[0m\n' "$*"; }
err()  { printf '\033[0;31m[apple-angle] %s\033[0m\n' "$*" >&2; }

[ -f "$LOCK" ] || { err "pin not found: $LOCK"; exit 1; }

WANTED=""
CHECK_ONLY=0
DEST="$ROOT/engine/third_party"
while [ $# -gt 0 ]; do
    case "$1" in
        --check) CHECK_ONLY=1; shift ;;
        --dest)  DEST="${2:-}"; shift 2 ;;
        -*)      err "unknown option: $1"; exit 2 ;;
        *)       WANTED="$WANTED $1"; shift ;;
    esac
done

# The lock is queried, never parsed with a regular expression: a JSON file read
# with grep is a file read incorrectly the day somebody reformats it. Arguments
# go through sys.argv rather than into the program text for the reason
# fetch-windows-angle.sh documents -- MSYS-style path rewriting applies to whole
# argv tokens, not to substrings inside a larger string.
lock_query() {
    "$(python_cmd)" -c '
import json, sys

with open(sys.argv[1], encoding="utf-8") as handle:
    lock = json.load(handle)
what = sys.argv[2]
if what == "release":
    print(lock["release"])
elif what == "platforms":
    print(" ".join(sorted(lock["targets"])))
elif what == "contents":
    print(" ".join(lock["targets"][sys.argv[3]]["contents"]))
elif what in ("asset", "sha256", "size_bytes"):
    print(lock["targets"][sys.argv[3]][what])
else:
    raise SystemExit("unknown query: " + what)
' "$LOCK" "$@"
}

digest_of() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        sha256sum "$1" | cut -d' ' -f1
    fi
}

if ! BASE_URL="$(lock_query release 2>/dev/null)"; then
    err "this pin carries no 'release' key, so there is nothing to fetch."
    err "The artifact half of contracts/artifact-manifest/apple-angle.lock.json is"
    err "written after scripts/build-angle-apple.sh has produced bytes to hash."
    exit 1
fi
[ -n "$WANTED" ] || WANTED="$(lock_query platforms)"

failures=0
for platform in $WANTED; do
    if ! asset="$(lock_query asset "$platform" 2>/dev/null)"; then
        err "$platform: not carried by the pin"
        failures=$((failures + 1)); continue
    fi
    want="$(lock_query sha256 "$platform")"
    unpacked="$DEST/angle-apple-$platform"

    # Present and complete is decided by the archive's hash, not by the tree:
    # comparing an unpacked directory against a pin means re-deriving what the
    # archive contained, and a check that reconstructs its own expectation is a
    # check that agrees with itself.
    # `angle-apple-cache`, not `.angle-apple-cache`: .gitignore covers
    # `engine/third_party/angle-apple-*/`, and a dot-prefixed sibling would sit
    # outside that pattern -- untracked build output showing up in `git status`
    # and in front of the repository hygiene gate.
    cached="$DEST/angle-apple-cache/$asset"
    if [ -f "$cached" ] && [ "$(digest_of "$cached")" = "$want" ] && [ -d "$unpacked" ]; then
        ok "$platform: already present and matches the pin"
        continue
    fi

    if [ "$CHECK_ONLY" = "1" ]; then
        err "$platform: missing or mismatched (checking only, nothing downloaded)"
        failures=$((failures + 1)); continue
    fi

    mkdir -p "$(dirname "$cached")"
    rm -f "$cached"
    info "downloading $platform ($asset)"
    if ! curl -fsSL --retry 3 --retry-delay 2 -o "$cached" "$BASE_URL/$asset"; then
        err "$platform: download failed from $BASE_URL/$asset"
        rm -f "$cached"
        failures=$((failures + 1)); continue
    fi

    have="$(digest_of "$cached")"
    if [ "$have" != "$want" ]; then
        err "$platform: sha256 mismatch after download"
        err "  expected $want"
        err "  got      $have"
        rm -f "$cached"
        failures=$((failures + 1)); continue
    fi

    rm -rf "$unpacked"
    mkdir -p "$DEST"
    tar -xzf "$cached" -C "$DEST"

    # What the pin says the archive holds, checked after unpacking rather than
    # trusted. The hash proves the bytes; this proves the bytes were the ones
    # this consumer needs -- a correctly hashed archive of the wrong two files
    # is exactly as useless as a corrupt one, and only one of those two failures
    # is loud on its own.
    incomplete=0
    for entry in $(lock_query contents "$platform"); do
        if [ ! -e "$unpacked/$entry" ]; then
            err "$platform: the archive verified but does not contain $entry"
            failures=$((failures + 1))
            incomplete=1
        fi
    done
    # ...and the layout ANGLE's own loader will search, ASKED OF THE RECIPE that
    # owns that rule rather than restated here. The check above proves the
    # archive holds the top-level entries the pin names; this proves what it
    # holds can be opened. On the iOS family the top-level entry is a framework
    # BUNDLE and the file ANGLE opens is inside it, so an archive that lost the
    # bundle's executable passes the `contents` check and fails at
    # eglInitialize -- with nothing of ours on the stack to say why.
    if ! layout="$(bash "$SCRIPT_DIR/build-angle-apple.sh" --print-loader-layout "$platform" 2>&1)"; then
        err "$platform: could not ask the recipe which layout ANGLE will search:"
        printf '%s\n' "$layout" | sed 's/^/  /' >&2
        failures=$((failures + 1))
        incomplete=1
        layout=""
    fi
    while read -r layout_target layout_path; do
        [ -n "$layout_path" ] || continue
        if [ ! -e "$unpacked/$layout_path" ]; then
            err "$platform: ANGLE opens $layout_target at '$layout_path', and the"
            err "  unpacked archive has nothing there"
            failures=$((failures + 1))
            incomplete=1
        fi
    done <<LAYOUT
$layout
LAYOUT

    # The success line only when there is a success to report. A green line
    # printed after a red one is how a failing step reads as a passing one in a
    # log somebody skims.
    if [ "$incomplete" -eq 0 ]; then
        ok "$platform: downloaded, verified and unpacked into $unpacked"
    fi
done

if [ "$failures" -gt 0 ]; then
    err "$failures platform(s) unavailable or unverified"
    exit 1
fi
ok "ANGLE for Apple verified against the pin"
