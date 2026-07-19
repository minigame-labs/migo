#!/usr/bin/env bash
# Cross-compile capi for Android and build the NativeActivity example APK.
#
# The APK is the acceptance vehicle for the Android C ABI: a host with no Java
# of its own, embedding migo through the public headers only.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE_DIR="$REPO_ROOT/engine"
TARGET="aarch64-linux-android"
ABI="arm64-v8a"

info() { echo -e "\033[0;36m[android-c-host] $*\033[0m"; }
err() { echo -e "\033[0;31m[android-c-host] $*\033[0m" >&2; }

ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Ndk}"
[[ -d "$ANDROID_NDK_HOME" ]] || { err "NDK not found at $ANDROID_NDK_HOME"; exit 1; }
NDK_BIN="$(echo "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin)"
export ANDROID_NDK_HOME
# skia-bindings reads ANDROID_NDK (not _HOME) when picking its toolchain.
export ANDROID_NDK="$ANDROID_NDK_HOME"
export PATH="$NDK_BIN:$PATH"

# engine/.cargo/config.toml names aarch64-linux-android-ar without a path; the
# NDK ships it as llvm-ar.
SHIM_DIR="$ENGINE_DIR/target/android-ar-shim"
mkdir -p "$SHIM_DIR"
ln -sf "$NDK_BIN/llvm-ar" "$SHIM_DIR/aarch64-linux-android-ar"
export PATH="$SHIM_DIR:$PATH"

V8_DIR="$ENGINE_DIR/third_party/rusty_v8/aarch64"
[[ -f "$V8_DIR/librusty_v8.a" ]] || { err "missing $V8_DIR/librusty_v8.a"; exit 1; }

info "building capi staticlib for $TARGET"
RUSTY_V8_ARCHIVE="$V8_DIR/librusty_v8.a" \
RUSTY_V8_SRC_BINDING_PATH="$V8_DIR/src_binding.rs" \
    cargo build -p capi --release --target "$TARGET" --manifest-path "$ENGINE_DIR/Cargo.toml"

STAGE="$REPO_ROOT/examples/c-host/android/src/main/jniLibs-static/$ABI"
mkdir -p "$STAGE"
cp "$ENGINE_DIR/target/$TARGET/release/libmigo_capi.a" "$STAGE/"
info "staged $(stat -c %s "$STAGE/libmigo_capi.a") bytes at $STAGE/libmigo_capi.a"

info "building the APK"
cd "$REPO_ROOT/platforms/android"
./gradlew --no-daemon :c-host-example:assembleDebug

APK="$(find "$REPO_ROOT/examples/c-host/android/build/outputs/apk" -name '*.apk' | head -1)"
[[ -n "$APK" ]] || { err "no APK produced"; exit 1; }
info "APK: $APK ($(stat -c %s "$APK") bytes)"
