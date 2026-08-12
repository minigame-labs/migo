#!/usr/bin/env bash
# Install the pinned OpenHarmony native SDK, verified against a committed hash.
#
# scripts/dev-setup-ohos.sh deliberately does not download: it says a URL baked
# into it "would rot silently", and it is right -- so it locates and asserts an
# SDK someone else installed, and its header carries the install recipe as a
# comment for a human to follow. That is fine for a workstation and useless for
# CI, where nobody has installed anything. This script is that recipe made
# executable and verifiable, with the version and hash in
# contracts/artifact-manifest/ohos-sdk.lock.json rather than in the script.
#
# The division of labour is deliberate: this script *obtains* an SDK,
# dev-setup-ohos.sh *validates and exports* one. Neither duplicates the other,
# and the build scripts keep calling only dev-setup-ohos.sh.
#
# Usage:
#   scripts/fetch-ohos-sdk.sh              # install to $HOME/ohos-sdk if absent
#   scripts/fetch-ohos-sdk.sh --check      # assert the installed SDK matches the pin
#   scripts/fetch-ohos-sdk.sh --force      # reinstall even if one is present
#
# Env:
#   OHOS_NDK_HOME   install prefix (default $HOME/ohos-sdk). The extracted tree
#                   puts `native/` directly under it, which is what
#                   dev-setup-ohos.sh probes for.
#   MIGO_OHOS_SDK_CACHE  where to keep the downloaded archive (default a temp
#                   directory removed on exit). Point it at a persistent path to
#                   avoid re-downloading 3.2 GB.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOCK="$REPO_ROOT/contracts/artifact-manifest/ohos-sdk.lock.json"

info() { echo -e "\033[0;36m[ohos-sdk] $*\033[0m"; }
ok()   { echo -e "\033[0;32m[ohos-sdk] $*\033[0m"; }
err()  { echo -e "\033[0;31m[ohos-sdk] $*\033[0m" >&2; }
die()  { err "$*"; exit 1; }

MODE="install"
case "${1:-}" in
    --check) MODE="check" ;;
    --force) MODE="force" ;;
    "") ;;
    *) die "unknown argument: $1 (expected --check or --force)" ;;
esac

[[ -f "$LOCK" ]] || die "pin not found: $LOCK"

# Every field comes from the lock file. A default here would let the script
# install something the repository never pinned.
read_pin() {
    python3 - "$LOCK" "$1" <<'PY'
import json, sys
node = json.load(open(sys.argv[1], encoding="utf-8"))
for key in sys.argv[2].split("."):
    node = node[key]
print(node)
PY
}

SDK_VERSION="$(read_pin sdk_version)"
API_VERSION="$(read_pin api_version)"
MIRROR="$(read_pin mirror)"
ARCHIVE_NAME="$(read_pin archive.name)"
ARCHIVE_SHA="$(read_pin archive.sha256)"
ARCHIVE_SIZE="$(read_pin archive.size_bytes)"
MEMBER_GLOB="$(read_pin component.member_glob)"

PREFIX="${OHOS_NDK_HOME:-$HOME/ohos-sdk}"

info "pinned SDK $SDK_VERSION (API $API_VERSION)"
info "install prefix: $PREFIX"

# ---- check ----------------------------------------------------------------
# What can be asserted about an already-extracted SDK is its own reported
# version, which is the one thing the pin also names. Delegated to
# dev-setup-ohos.sh rather than parsed here from the SDK's metadata a second
# time: two readers of one fact is how they come to disagree.
installed_version() {
    bash "$SCRIPT_DIR/dev-setup-ohos.sh" 2>/dev/null \
        | sed -n 's/.*SDK version: \([0-9.]*\).*/\1/p' | head -1
}

if [[ "$MODE" == "check" ]]; then
    bash "$SCRIPT_DIR/dev-setup-ohos.sh" --check >/dev/null 2>&1 \
        || die "no usable OpenHarmony SDK found; run this script with no arguments to install one"
    found="$(installed_version)"
    [[ -n "$found" ]] || die "an SDK is present but dev-setup-ohos.sh reported no version"
    if [[ "$found" != "$SDK_VERSION" ]]; then
        err "installed SDK is $found but the pin is $SDK_VERSION."
        err "Every OpenHarmony artifact records build_sdk, so building with a different"
        err "SDK than the pin publishes a package whose toolchain identity is not the one"
        err "this repository declares. Either reinstall with --force or bump the pin"
        err "deliberately in $LOCK."
        exit 1
    fi
    ok "installed SDK $found matches the pin"
    exit 0
fi

if [[ "$MODE" == "install" ]] && bash "$SCRIPT_DIR/dev-setup-ohos.sh" --check >/dev/null 2>&1; then
    found="$(installed_version)"
    if [[ "$found" == "$SDK_VERSION" ]]; then
        ok "SDK $found already installed and matches the pin; nothing to do"
        exit 0
    fi
    die "an SDK is installed ($found) but the pin is $SDK_VERSION; re-run with --force to replace it"
fi

# ---- download -------------------------------------------------------------
if [[ -n "${MIGO_OHOS_SDK_CACHE:-}" ]]; then
    WORK="$MIGO_OHOS_SDK_CACHE"
    mkdir -p "$WORK"
else
    WORK="$(mktemp -d)"
    trap 'rm -rf "$WORK"' EXIT
fi
ARCHIVE="$WORK/$ARCHIVE_NAME"

if [[ -f "$ARCHIVE" ]] && [[ "$(stat -c%s "$ARCHIVE")" == "$ARCHIVE_SIZE" ]]; then
    info "reusing the archive already in $WORK"
else
    info "downloading $ARCHIVE_NAME ($((ARCHIVE_SIZE / 1048576)) MB) from the pinned mirror"
    # -C - resumes a partial file, which matters at this size: a dropped
    # connection 3 GB in should not restart from zero.
    curl -fL --retry 3 --retry-delay 2 -C - -o "$ARCHIVE" "$MIRROR/$ARCHIVE_NAME" \
        || die "download failed from $MIRROR/$ARCHIVE_NAME"
fi

# ---- verify before use ----------------------------------------------------
# Before extracting, not after. Extracting first would mean an unzip of
# unverified bytes, and the whole point of a committed hash is that nothing
# untrusted is acted on.
actual_size="$(stat -c%s "$ARCHIVE")"
[[ "$actual_size" == "$ARCHIVE_SIZE" ]] \
    || die "archive is $actual_size bytes, pin says $ARCHIVE_SIZE -- refusing to extract"
info "verifying sha256 against the committed pin"
actual_sha="$(sha256sum "$ARCHIVE" | cut -d' ' -f1)"
if [[ "$actual_sha" != "$ARCHIVE_SHA" ]]; then
    err "sha256 mismatch -- refusing to extract."
    err "  expected (pinned): $ARCHIVE_SHA"
    err "  actual            : $actual_sha"
    err "The mirror's contents changed under the same filename, or the download is"
    err "corrupt. Do not 'fix' this by updating the pin without establishing which."
    exit 1
fi
ok "archive matches the pin"

# ---- extract ---------------------------------------------------------------
# One member out of the archive, located by glob. --wildcards is required for
# GNU tar to treat the pattern as one.
info "extracting $MEMBER_GLOB"
rm -rf "$WORK/extracted"
mkdir -p "$WORK/extracted"
tar -xzf "$ARCHIVE" -C "$WORK/extracted" --wildcards "$MEMBER_GLOB" \
    || die "the archive does not contain $MEMBER_GLOB -- the tar layout changed, so the glob in $LOCK needs updating"

mapfile -t COMPONENTS < <(find "$WORK/extracted" -name '*.zip' -type f | sort)
(( ${#COMPONENTS[@]} == 1 )) \
    || die "expected exactly one component zip, found ${#COMPONENTS[@]}: ${COMPONENTS[*]}"

info "unpacking $(basename "${COMPONENTS[0]}") into $PREFIX"
rm -rf "$PREFIX"
mkdir -p "$PREFIX"
unzip -q "${COMPONENTS[0]}" -d "$PREFIX" || die "unzip failed"

# ---- assert the result is what the build scripts probe for -----------------
# dev-setup-ohos.sh looks for `native/` directly under the prefix. An archive
# whose zip carries an extra top-level directory would extract "successfully"
# and leave an SDK nothing can find, so the shape is asserted here rather than
# discovered by a build failure later.
if [[ ! -d "$PREFIX/native" ]]; then
    nested="$(find "$PREFIX" -maxdepth 2 -type d -name native | head -1)"
    [[ -n "$nested" ]] || die "no native/ directory anywhere under $PREFIX after extraction"
    info "flattening: the component zip nested native/ one level deeper"
    mv "$nested"/../* "$PREFIX"/ 2>/dev/null || true
    [[ -d "$PREFIX/native" ]] || die "could not place native/ directly under $PREFIX"
fi

bash "$SCRIPT_DIR/fetch-ohos-sdk.sh" --check
