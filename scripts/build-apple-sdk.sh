#!/usr/bin/env bash
# =============================================================================
# Build the Apple SDK: a Rust static library per slice, assembled into
# MigoEngine.xcframework alongside the C ABI headers and a module map.
#
# The deployment targets are READ from contracts/apple/deployment-floor.json,
# not written here. A copy in this file would be a second place the decision
# lives, and the one that silently wins on the build machine. The contract gate
# checks this by asking the script (`--print-deployment-target`) rather than by
# grepping it, so a copy reintroduced later would still have to agree.
#
# WHAT THIS DOES NOT DO, AND WHY IT SAYS SO.
# Building requires macOS: Rust's Apple targets need Xcode's linker and SDKs,
# and xcframework assembly needs xcodebuild. On any other host this exits with
# a message naming what is missing. It does not stub, skip, or report success --
# a build script that "passes" without producing bytes is how a Windows SDK
# shipped that loaded, resolved every entry point, and could attach nothing.
# =============================================================================
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE_DIR="$REPO_ROOT/engine"
CONTRACT="$REPO_ROOT/contracts/apple/deployment-floor.json"
WEBCONTENT_SRC="$REPO_ROOT/platforms/apple/WebContent/PerformancePlus"
WEBCONTENT_DEST="$REPO_ROOT/platforms/apple/Sources/MigoApplePerformancePlus/Resources"
FRAMEWORKS_DIR="$REPO_ROOT/platforms/apple/Frameworks"

BUILD_ROOT="${MIGO_APPLE_BUILD_ROOT:-/tmp/migo-apple-build}"

PLATFORM=""
CONFIGURATION="Debug"
CODE_SIGNING="off"
PRINT_TARGET=""

err()  { printf '\033[0;31m[apple-sdk] %s\033[0m\n' "$*" >&2; }
ok()   { printf '\033[0;32m[apple-sdk] %s\033[0m\n' "$*"; }
info() { printf '\033[0;36m[apple-sdk] %s\033[0m\n' "$*"; }

usage() {
    cat <<'USAGE'
usage: build-apple-sdk.sh --platform <ios|ios-simulator|macos>
                          [--configuration Debug|Release]
                          [--code-signing on|off]
       build-apple-sdk.sh --print-deployment-target <ios|macos>

  --print-deployment-target  Print the deployment target this build would use,
                             read from contracts/apple/deployment-floor.json,
                             and exit. Runs on any host: it is how the contract
                             gate checks the script's real behaviour instead of
                             grepping it for a number.

Slices, and why each exists:
  ios              aarch64-apple-ios          device
  ios-simulator    aarch64-apple-ios-sim,     Apple silicon and Intel Macs both
                   x86_64-apple-ios           run the simulator
  macos            aarch64-apple-darwin,      Apple silicon and Intel, as real
                   x86_64-apple-darwin        slices -- Rosetta is not a slice

Output: $MIGO_APPLE_BUILD_ROOT (default /tmp/migo-apple-build), with the
finished xcframework copied to platforms/apple/Frameworks/.
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --platform)      PLATFORM="${2:-}"; shift 2 ;;
        --configuration) CONFIGURATION="${2:-}"; shift 2 ;;
        --code-signing)  CODE_SIGNING="${2:-}"; shift 2 ;;
        --print-deployment-target) PRINT_TARGET="${2:-}"; shift 2 ;;
        -h|--help)       usage; exit 0 ;;
        *)               err "unknown argument: $1"; usage >&2; exit 2 ;;
    esac
done

# ---------------------------------------------------------------------------
# The floor, read from the contract
# ---------------------------------------------------------------------------

read_deployment_target() {
    local platform="$1"
    python3 - "$CONTRACT" "$platform" <<'PY'
import json
import sys

path, platform = sys.argv[1], sys.argv[2]
with open(path, encoding="utf-8") as handle:
    contract = json.load(handle)
entry = (contract.get("platforms") or {}).get(platform)
if entry is None:
    print(f"unknown platform {platform!r}", file=sys.stderr)
    raise SystemExit(1)
target = entry.get("deployment_target")
if not target:
    print(f"{platform} declares no deployment_target", file=sys.stderr)
    raise SystemExit(1)
print(target)
PY
}

if [ -n "$PRINT_TARGET" ]; then
    read_deployment_target "$PRINT_TARGET" || exit 1
    exit 0
fi

if [ -z "$PLATFORM" ]; then
    err "--platform is required"
    usage >&2
    exit 2
fi

case "$CONFIGURATION" in
    Debug|Release) ;;
    *) err "--configuration must be Debug or Release"; exit 2 ;;
esac

case "$CODE_SIGNING" in
    on|off) ;;
    *) err "--code-signing must be on or off"; exit 2 ;;
esac

case "$PLATFORM" in
    ios)           RUST_TARGETS=(aarch64-apple-ios); FLOOR_PLATFORM="ios" ;;
    ios-simulator) RUST_TARGETS=(aarch64-apple-ios-sim x86_64-apple-ios); FLOOR_PLATFORM="ios" ;;
    macos)         RUST_TARGETS=(aarch64-apple-darwin x86_64-apple-darwin); FLOOR_PLATFORM="macos" ;;
    *) err "--platform must be ios, ios-simulator or macos"; exit 2 ;;
esac

DEPLOYMENT_TARGET="$(read_deployment_target "$FLOOR_PLATFORM")" || exit 1

# ---------------------------------------------------------------------------
# Host requirements, checked before anything is created
# ---------------------------------------------------------------------------

if [ "$(uname -s)" != "Darwin" ]; then
    err "Apple slices need macOS: Rust's Apple targets require Xcode's linker"
    err "and SDKs, and xcframework assembly requires xcodebuild."
    err ""
    err "This host is $(uname -s). Refusing to report a build that did not happen."
    err "Everything except the build is still checkable here:"
    err "  bash scripts/test-apple-deployment-floor-contract.sh"
    err "  bash scripts/test-apple-profile-policy-contract.sh"
    err "  bash scripts/test-c-abi-surface-candidate.sh"
    exit 1
fi

missing=0
for tool in xcodebuild lipo cargo; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        err "required tool not on PATH: $tool"
        missing=1
    fi
done
[ "$missing" -eq 0 ] || exit 1

for target in "${RUST_TARGETS[@]}"; do
    if ! rustup target list --installed 2>/dev/null | grep -qx "$target"; then
        err "Rust target not installed: $target"
        err "  rustup target add $target"
        exit 1
    fi
done

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

STAGE="$BUILD_ROOT/$PLATFORM-$CONFIGURATION"
rm -rf "$STAGE"
mkdir -p "$STAGE/libs" "$STAGE/headers/migo"

info "platform            $PLATFORM ($CONFIGURATION)"
info "deployment target   $DEPLOYMENT_TARGET (from contracts/apple/deployment-floor.json)"
info "slices              ${RUST_TARGETS[*]}"

cargo_profile_flag=()
profile_dir="debug"
if [ "$CONFIGURATION" = "Release" ]; then
    cargo_profile_flag=(--release)
    profile_dir="release"
fi

# cd into engine/ deliberately: rust-toolchain.toml and .cargo/config.toml are
# both resolved from the working directory, so building with --manifest-path
# from the repo root would silently use the machine's default toolchain.
cd "$ENGINE_DIR" || exit 1

for target in "${RUST_TARGETS[@]}"; do
    info "cargo build $target"
    case "$FLOOR_PLATFORM" in
        ios)   export IPHONEOS_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET" ;;
        macos) export MACOSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET" ;;
    esac
    if ! cargo build -p migo-capi --target "$target" --locked "${cargo_profile_flag[@]}"; then
        err "cargo build failed for $target"
        exit 1
    fi
    cp "target/$target/$profile_dir/libmigo.a" "$STAGE/libs/libmigo-$target.a" || exit 1
done

# One archive per xcframework slice group. Device and simulator must stay
# separate archives -- an xcframework rejects a fat archive that mixes them,
# and lipo will happily produce one.
if [ "${#RUST_TARGETS[@]}" -gt 1 ]; then
    info "lipo ${#RUST_TARGETS[@]} slices into one archive"
    lipo -create "$STAGE"/libs/libmigo-*.a -output "$STAGE/libmigo.a" || exit 1
else
    cp "$STAGE/libs/libmigo-${RUST_TARGETS[0]}.a" "$STAGE/libmigo.a" || exit 1
fi

# Headers travel with the binary, so there is no vendored second copy to drift.
cp "$REPO_ROOT"/include/migo/*.h "$STAGE/headers/migo/" || exit 1
mkdir -p "$STAGE/headers/migo/platform"
cp "$REPO_ROOT"/include/migo/platform/*.h "$STAGE/headers/migo/platform/" || exit 1

cat > "$STAGE/headers/module.modulemap" <<'MODULEMAP'
module MigoEngine {
    umbrella header "migo/migo.h"
    header "migo/external_frames.h"
    header "migo/platform/ios.h"
    header "migo/platform/macos.h"
    export *
}
MODULEMAP

mkdir -p "$FRAMEWORKS_DIR"
XCFRAMEWORK="$FRAMEWORKS_DIR/MigoEngine.xcframework"
rm -rf "$XCFRAMEWORK"
if ! xcodebuild -create-xcframework \
        -library "$STAGE/libmigo.a" \
        -headers "$STAGE/headers" \
        -output "$XCFRAMEWORK"; then
    err "xcodebuild -create-xcframework failed"
    exit 1
fi

# ---------------------------------------------------------------------------
# The WebContent producer bundle
# ---------------------------------------------------------------------------

if [ -d "$WEBCONTENT_SRC" ]; then
    info "staging the WebContent producer"
    rm -rf "${WEBCONTENT_DEST:?}"/*
    mkdir -p "$WEBCONTENT_DEST"
    # Copied rather than bundled while the producer is still source-only. The
    # bundling step lands with the producer itself; doing it now would be a
    # build step over nothing.
    cp -R "$WEBCONTENT_SRC"/. "$WEBCONTENT_DEST/" || exit 1
fi

if [ "$CODE_SIGNING" = "on" ]; then
    info "code signing requested; the signing identity and entitlements are the"
    info "host application's, not this SDK's. macOS V8 additionally requires"
    info "com.apple.security.cs.allow-jit in the embedding app's entitlements."
fi

ok "built $XCFRAMEWORK"
ok "deployment target $DEPLOYMENT_TARGET, slices ${RUST_TARGETS[*]}"
exit 0
