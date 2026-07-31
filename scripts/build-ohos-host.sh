#!/usr/bin/env bash
# =============================================================================
# Stage migo into the OpenHarmony host project and build the HAP.
#
# The host in platforms/openharmony is an ordinary DevEco project that consumes
# the C SDK: it sees only the public headers and links libmigo_capi.a into its
# own libmigohost.so. That is the same relationship the Android NativeActivity
# host has with the Android C SDK, and keeping it that way is what makes the
# host a test of the SDK rather than an extension of it.
#
# Usage:
#   scripts/build-ohos-host.sh [--arch x86_64|aarch64] [--no-hap]
#
# Env:
#   OHOS_NDK_HOME    OpenHarmony SDK (probed via dev-setup-ohos.sh)
#   DEVECO_HOME      DevEco Studio install (default: the standard Windows path
#                    reached through /mnt/c, since the emulator and hvigor live
#                    on the Windows side while the engine is built in WSL)
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
HOST_DIR="$REPO_ROOT/platforms/openharmony"

info() { echo -e "\033[0;36m[ohos-host] $*\033[0m"; }
err()  { echo -e "\033[0;31m[ohos-host] $*\033[0m" >&2; }
ok()   { echo -e "\033[0;32m[ohos-host] $*\033[0m"; }

ARCH="x86_64"
BUILD_HAP=1
while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch) ARCH="$2"; shift 2 ;;
        --no-hap) BUILD_HAP=0; shift ;;
        *) err "unknown argument: $1"; exit 2 ;;
    esac
done
TRIPLE="$ARCH-unknown-linux-ohos"

# ---- build the static library the host links --------------------------------
V8_DIR="$REPO_ROOT/engine/third_party/rusty_v8/$ARCH-linux-ohos"
if [[ ! -f "$V8_DIR/librusty_v8.a" ]]; then
    err "missing $V8_DIR/librusty_v8.a"
    err "build it first: scripts/build-v8-ohos.sh $ARCH"
    exit 1
fi

eval "$(bash "$SCRIPT_DIR/dev-setup-ohos.sh" | grep '^export ')"
[[ -n "${OHOS_SDK_NATIVE:-}" ]] || { err "OpenHarmony SDK not usable"; exit 1; }

STATIC_LIB="$REPO_ROOT/engine/target/$TRIPLE/release/libmigo_capi.a"
info "building migo-capi for $TRIPLE"
(
    cd "$REPO_ROOT/engine"
    # From engine/ so rust-toolchain.toml applies; profile-slim because the
    # full profile needs ALSA, which OpenHarmony does not have.
    RUSTY_V8_ARCHIVE="$V8_DIR/librusty_v8.a" \
    RUSTY_V8_SRC_BINDING_PATH="$V8_DIR/src_binding.rs" \
        cargo build -p migo-capi --release \
            --no-default-features --features profile-slim \
            --target "$TRIPLE"
)
[[ -f "$STATIC_LIB" ]] || { err "no static library produced"; exit 1; }

# ---- stage it where CMakeLists.txt expects ----------------------------------
CPP_DIR="$HOST_DIR/entry/src/main/cpp"
mkdir -p "$CPP_DIR/libs/$ARCH" "$CPP_DIR/migo-include"
cp "$STATIC_LIB" "$CPP_DIR/libs/$ARCH/"
rm -rf "$CPP_DIR/migo-include/migo"
cp -r "$REPO_ROOT/include/migo" "$CPP_DIR/migo-include/"
ok "staged $(stat -c %s "$STATIC_LIB") bytes + public headers"

if [[ $BUILD_HAP -eq 0 ]]; then
    info "--no-hap given; stopping before the DevEco build"
    exit 0
fi

# ---- build the HAP ----------------------------------------------------------
# hvigor lives on the Windows side together with the emulator. The repository
# is on a WSL path, and cmd.exe refuses a UNC working directory, so hvigorw is
# invoked with an explicit project directory rather than by chdir-ing into it.
DEVECO_HOME="${DEVECO_HOME:-/mnt/c/Program Files/Huawei/DevEco Studio}"
HVIGOR="$DEVECO_HOME/tools/hvigor/bin/hvigorw.js"
if [[ ! -f "$HVIGOR" ]]; then
    err "hvigor not found at $HVIGOR"
    err "set DEVECO_HOME to the DevEco Studio installation"
    exit 1
fi
info "building the HAP with hvigor"
info "  project: $HOST_DIR"
err "hvigor must run on the Windows side; invoke it from there against"
err "  $(wslpath -w "$HOST_DIR" 2>/dev/null || echo "$HOST_DIR")"
err "this script has staged everything it needs."
exit 0
