#!/usr/bin/env bash
# The C ABI must compile for every platform it claims to support.
#
# capi was desktop-only for two slices and nothing noticed, because no gate ever
# built it for Android. A cross `check` is cheap and catches the whole class of
# regression: naming a platform type directly from an entry point, or adding a
# call only one backend can satisfy.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE_DIR="$REPO_ROOT/engine"
TARGET="${MIGO_ANDROID_TARGET:-aarch64-linux-android}"
ARCH_DIR="aarch64"

info() { echo -e "\033[0;36m[capi-platform] $*\033[0m"; }
err() { echo -e "\033[0;31m[capi-platform] $*\033[0m" >&2; }

ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Ndk}"
if [[ ! -d "$ANDROID_NDK_HOME" ]]; then
    err "NDK not found at $ANDROID_NDK_HOME; set ANDROID_NDK_HOME"
    exit 1
fi
NDK_BIN="$(echo "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin)"
export PATH="$NDK_BIN:$PATH"

# engine/.cargo/config.toml names aarch64-linux-android-ar without a path, and
# the NDK ships it as llvm-ar. Provide the expected name here rather than
# editing that shared config, which the AAR build also relies on.
SHIM_DIR="$(mktemp -d)"
trap 'rm -rf "$SHIM_DIR"' EXIT
ln -sf "$NDK_BIN/llvm-ar" "$SHIM_DIR/aarch64-linux-android-ar"
export PATH="$SHIM_DIR:$PATH"

V8_DIR="$ENGINE_DIR/third_party/rusty_v8/$ARCH_DIR"
if [[ ! -f "$V8_DIR/librusty_v8.a" ]]; then
    err "missing $V8_DIR/librusty_v8.a"
    exit 1
fi

# Set inline, never exported: leaking these into a host build makes it link the
# Android archive and fail confusingly.
info "cross-checking capi for $TARGET"
RUSTY_V8_ARCHIVE="$V8_DIR/librusty_v8.a" \
RUSTY_V8_SRC_BINDING_PATH="$V8_DIR/src_binding.rs" \
    cargo check -p capi --target "$TARGET" --manifest-path "$ENGINE_DIR/Cargo.toml"

info "OK: the C ABI compiles for $TARGET"
