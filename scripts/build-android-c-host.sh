#!/usr/bin/env bash
# Cross-compile capi for Android and build the NativeActivity example APK.
#
# The APK is the acceptance vehicle for the Android C ABI: a host with no Java
# of its own, embedding migo through the public headers only.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/lib/v8-materialise.sh
source "$SCRIPT_DIR/lib/v8-materialise.sh"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE_DIR="$REPO_ROOT/engine"

# The ABI is an argument because the only Android hardware this project can
# always reach is an emulator, and the emulator that runs at usable speed here
# is x86_64 (KVM). A harness that only builds for the phone ABI is a harness
# that cannot be run.
ABI="${1:-arm64-v8a}"
case "$ABI" in
arm64-v8a) TARGET="aarch64-linux-android" ;;
x86_64) TARGET="x86_64-linux-android" ;;
*) echo "usage: $0 [arm64-v8a|x86_64]" >&2; exit 2 ;;
esac

info() { echo -e "\033[0;36m[android-c-host] $*\033[0m"; }
err() { echo -e "\033[0;31m[android-c-host] $*\033[0m" >&2; }

# shellcheck source=scripts/lib/android-ndk.sh
source "$SCRIPT_DIR/lib/android-ndk.sh"
android_ndk_read_pin "$REPO_ROOT/contracts/artifact-manifest/android-v8.lock.json" || exit 1
android_ndk_resolve || { err "cannot resolve the pinned Android NDK"; exit 1; }
NDK_BIN="$(echo "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin)"
export ANDROID_NDK_HOME
# skia-bindings reads ANDROID_NDK (not _HOME) when picking its toolchain.
export ANDROID_NDK="$ANDROID_NDK_HOME"
export PATH="$NDK_BIN:$PATH"

# engine/.cargo/config.toml names <triple>-ar without a path; the NDK ships it
# as llvm-ar.
SHIM_DIR="$ENGINE_DIR/target/android-ar-shim"
mkdir -p "$SHIM_DIR"
ln -sf "$NDK_BIN/llvm-ar" "$SHIM_DIR/${TARGET}-ar"
export PATH="$SHIM_DIR:$PATH"

V8_DIR="$ENGINE_DIR/third_party/rusty_v8/$TARGET"
# Verified against its component manifest, then used from a content-addressed path. No
# separate existence check below it: the materialiser refuses a missing archive by name, so
# one here would be dead code shaped like a guard.
v8_materialise "$V8_DIR" "$ENGINE_DIR/target/v8-materialised" || exit 1

info "building capi staticlib for $TARGET"
# Built from inside engine/ so engine/rust-toolchain.toml applies -- it is
# resolved from the working directory, not from --manifest-path, so building
# from the repository root silently used the machine's default toolchain
# instead of the pinned one.
#
# ⚠ That move also activates engine/.cargo/config.toml's `[env]`, which sets a
# bare CC=clang-18 for every target; cc-rs would then build the C dependencies
# for Android with the host compiler and fail on bits/libc-header-start.h. It
# does not fail on a machine that exports a CC pointing at the NDK, so the
# failure only appears on a clean runner. cc-rs resolves CC_<target> before bare
# CC, so pinning the target-scoped names settles it.
NDK_API="${MIGO_ANDROID_API:-26}"
NDK_CC="$NDK_BIN/${TARGET}${NDK_API}-clang"
[[ -x "$NDK_CC" ]] || { err "NDK clang driver not found: $NDK_CC"; exit 1; }
TARGET_U="${TARGET//-/_}"
(
    cd "$ENGINE_DIR"
    env \
        "CC_${TARGET_U}=$NDK_CC" \
        "CXX_${TARGET_U}=${NDK_CC}++" \
        "AR_${TARGET_U}=$NDK_BIN/llvm-ar" \
        RUSTY_V8_ARCHIVE="$V8_MATERIALISED_ARCHIVE" \
        RUSTY_V8_SRC_BINDING_PATH="$V8_MATERIALISED_BINDING" \
        cargo build -p migo-capi --release --target "$TARGET"
)

STAGE="$REPO_ROOT/tests/c_host/android/src/main/jniLibs-static/$ABI"
mkdir -p "$STAGE"
cp "$ENGINE_DIR/target/$TARGET/release/libmigo_capi.a" "$STAGE/"
info "staged $(stat -c %s "$STAGE/libmigo_capi.a") bytes at $STAGE/libmigo_capi.a"

info "building the APK"
cd "$REPO_ROOT/platforms/android"
# The Gradle project builds only the ABIs this script staged a staticlib for;
# CMake links it by path, so an unstaged ABI fails deep in the linker instead.
./gradlew --no-daemon "-PmigoAbis=$ABI" :c-host-example:assembleDebug

APK="$(find "$REPO_ROOT/tests/c_host/android/build/outputs/apk" -name '*.apk' | head -1)"
[[ -n "$APK" ]] || { err "no APK produced"; exit 1; }
info "APK: $APK ($(stat -c %s "$APK") bytes)"
