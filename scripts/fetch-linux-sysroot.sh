#!/usr/bin/env bash
# ============================================================
# Materialise the Debian bullseye sysroot the Linux SDK builds against.
# Location: scripts/fetch-linux-sysroot.sh
#
# scripts/lib/linux-sysroot.sh used to resolve the sysroot inside a sibling
# ../rusty_v8_src checkout, so a Linux package could only be produced on a machine
# that happened to have one -- which is the whole reason the packaging gate is
# documented as release-machine-only. Nothing about the sysroot needs that
# checkout: Chromium publishes it as a tarball addressed by its own sha256.
#
# The recipe is engine/third_party/linux-sysroot/sysroots.json, a verbatim copy of
# Chromium's pin file, and verbatim is load-bearing twice. It names the tarball,
# its URL prefix and its sha256; and its *own* sha256 is the sysroot identity
# recorded in every Linux package manifest and in the linux-gnu V8 component
# manifest, which scripts/build-linux-sdk.sh requires to be equal. So a copy that
# drifted from the V8 checkout's cannot mis-build silently -- the SDK build stops
# and prints both identities.
#
# Because the tarball is addressed by its sha256, the URL and the expected hash
# are the same constant: bytes that fail the check could not have been served from
# that path in the first place.
#
# Usage: bash scripts/fetch-linux-sysroot.sh [--check] [--key KEY] [--dest DIR]
#   --check     verify what is present; download nothing
#   --key KEY   sysroots.json key (default bullseye_amd64; see below)
#   --dest DIR  install parent for the sysroot tree (default
#               engine/third_party/linux-sysroot, where the recipe is read from
#               regardless)
# ============================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

SYSROOT_HOME="${MIGO_SYSROOT_HOME:-$ROOT/engine/third_party/linux-sysroot}"
RECIPE="${MIGO_SYSROOT_RECIPE:-$SYSROOT_HOME/sysroots.json}"
KEY="bullseye_amd64"
DEST="$SYSROOT_HOME"
CHECK_ONLY=0

# The recipe advertises seven other architectures, and none of them is usable here:
# the library path this script checks for, scripts/lib/linux-sysroot.sh's link
# directory and scripts/build-linux-sdk.sh's target triple are all x86_64. A key
# that downloaded a valid arm64 tarball and then failed on an x86_64 path would
# read as a broken download rather than an unsupported request.
SUPPORTED_KEY="bullseye_amd64"
SYSROOT_LIB_TRIPLET="x86_64-linux-gnu"

info() { printf '\033[0;36m[linux-sysroot] %s\033[0m\n' "$*"; }
ok()   { printf '\033[0;32m[linux-sysroot] %s\033[0m\n' "$*"; }
err()  { printf '\033[0;31m[linux-sysroot] %s\033[0m\n' "$*" >&2; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --check) CHECK_ONLY=1; shift ;;
        --key)   KEY="$2"; shift 2 ;;
        --dest)  DEST="$2"; shift 2 ;;
        *)
            err "unknown argument: $1"
            err "usage: fetch-linux-sysroot.sh [--check] [--key KEY] [--dest DIR]"
            exit 2
            ;;
    esac
done

if [[ "$KEY" != "$SUPPORTED_KEY" ]]; then
    err "unsupported sysroot key: $KEY"
    err "Only $SUPPORTED_KEY is usable: the engine's Linux target, its link directory"
    err "and this script's completeness check are all $SYSROOT_LIB_TRIPLET."
    exit 2
fi

if [[ ! -f "$RECIPE" ]]; then
    err "sysroot recipe not found: $RECIPE"
    err "It is a verbatim copy of <v8-checkout>/build/linux/sysroot_scripts/sysroots.json."
    exit 1
fi

read -r TARBALL SHA256 URL_PREFIX SYSROOT_DIR < <(python3 - "$RECIPE" "$KEY" <<'PY'
import json
import sys

recipe, key = sys.argv[1], sys.argv[2]
with open(recipe, encoding="utf-8") as handle:
    entries = json.load(handle)
if key not in entries:
    sys.exit(f"no sysroot named {key!r} in {recipe}; known: {', '.join(sorted(entries))}")
entry = entries[key]
missing = [f for f in ("Tarball", "Sha256Sum", "URL", "SysrootDir") if not entry.get(f)]
if missing:
    sys.exit(f"{key} is missing {', '.join(missing)}")
print(entry["Tarball"], entry["Sha256Sum"], entry["URL"], entry["SysrootDir"])
PY
)

SYSROOT="$DEST/$SYSROOT_DIR"
URL="$URL_PREFIX/$SHA256"
# Chromium's install-sysroot.py writes the URL it installed into `.stamp` and
# returns early when it still matches. Writing the same marker means a tree
# installed by either tool is recognised by the other.
STAMP="$SYSROOT/.stamp"
# The one library whose absence is not a cosmetic gap: without it `-lstdc++`
# falls through to the host GCC's copy, which needs glibc symbols the sysroot
# does not define, and the link dies in a wall of undefined-shlib errors.
WITNESS="$SYSROOT/usr/lib/$SYSROOT_LIB_TRIPLET/libstdc++.so.6"

if [[ -f "$STAMP" && "$(cat "$STAMP")" == "$URL" && -f "$WITNESS" ]]; then
    ok "$KEY: already installed at $SYSROOT"
    exit 0
fi

if [[ "$CHECK_ONLY" == "1" ]]; then
    if [[ ! -d "$SYSROOT" ]]; then
        err "$KEY: not installed at $SYSROOT"
    elif [[ ! -f "$STAMP" ]]; then
        err "$KEY: $SYSROOT exists but carries no .stamp, so what it holds is unknown"
    elif [[ "$(cat "$STAMP")" != "$URL" ]]; then
        err "$KEY: installed from a different pin than the recipe names"
        err "  recipe:    $URL"
        err "  installed: $(cat "$STAMP")"
    else
        err "$KEY: .stamp matches but $WITNESS is missing, so the tree is incomplete"
    fi
    err "run without --check to install it"
    exit 1
fi

mkdir -p "$DEST"
TMP_TARBALL="$(mktemp "$DEST/.$SYSROOT_DIR.tar.xz.XXXXXX")"
TMP_TREE="$(mktemp -d "$DEST/.$SYSROOT_DIR.XXXXXX")"
cleanup() {
    rm -f -- "$TMP_TARBALL"
    if [[ -n "$TMP_TREE" ]]; then
        rm -rf -- "$TMP_TREE"
    fi
}
trap cleanup EXIT

info "$KEY: downloading $TARBALL"
if ! curl -fsSL --retry 3 --retry-delay 2 -o "$TMP_TARBALL" "$URL"; then
    err "$KEY: download failed from $URL"
    exit 1
fi

HAVE="$(sha256sum "$TMP_TARBALL" | cut -d' ' -f1)"
if [[ "$HAVE" != "$SHA256" ]]; then
    err "$KEY: sha256 mismatch after download"
    err "  expected $SHA256"
    err "  got      $HAVE"
    exit 1
fi
info "$KEY: sha256 verified against the recipe"

# `m` matches Chromium's own extraction: sysroot file mtimes are not part of what
# the compiler reads, and restoring them from a 2025 tarball makes every
# freshness check on the tree lie.
tar mxf "$TMP_TARBALL" -C "$TMP_TREE"
if [[ ! -f "$TMP_TREE/usr/lib/$SYSROOT_LIB_TRIPLET/libstdc++.so.6" ]]; then
    err "$KEY: the extracted tree has no libstdc++.so.6; the tarball is not a sysroot"
    exit 1
fi
printf '%s' "$URL" > "$TMP_TREE/.stamp"

# Replace in one move so an interrupted extraction never leaves a partial tree at
# the path the build resolves.
rm -rf -- "$SYSROOT"
mv -- "$TMP_TREE" "$SYSROOT"
TMP_TREE=""

ok "$KEY: installed at $SYSROOT"
