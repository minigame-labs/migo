#!/usr/bin/env bash
# Build the Android C ABI package: cross-compile capi to a static library and
# stage a package a third-party NDK host can consume through CMake, with a
# per-ABI artifact manifest.
#
# This is the Android counterpart of build-linux-sdk.sh. Android ships a static
# library rather than a versioned shared object -- an NDK host links it into its
# own .so -- and embeds a V8 startup snapshot, where Linux embeds none.
#
# Usage: scripts/build-android-sdk.sh [--arch aarch64|x86_64] [--prefix DIR]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE_DIR="$REPO_ROOT/engine"

ARCH="aarch64"
PREFIX=""
VERSION="0.1.0"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch) ARCH="$2"; shift 2 ;;
        --prefix) PREFIX="$2"; shift 2 ;;
        --version) VERSION="$2"; shift 2 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

case "$ARCH" in
    aarch64) TARGET="aarch64-linux-android"; ABI="arm64-v8a" ;;
    x86_64)  TARGET="x86_64-linux-android";  ABI="x86_64" ;;
    *) echo "unsupported arch: $ARCH (expected aarch64 or x86_64)" >&2; exit 2 ;;
esac
[[ -n "$PREFIX" ]] || PREFIX="$REPO_ROOT/dist/migo-android-$ABI"

info() { echo -e "\033[0;36m[android-sdk] $*\033[0m"; }
err() { echo -e "\033[0;31m[android-sdk] $*\033[0m" >&2; }

ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Ndk}"
[[ -d "$ANDROID_NDK_HOME" ]] || { err "NDK not found at $ANDROID_NDK_HOME"; exit 1; }
NDK_BIN="$(echo "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin)"
export ANDROID_NDK_HOME
# skia-bindings reads ANDROID_NDK (not _HOME) when picking its toolchain.
export ANDROID_NDK="$ANDROID_NDK_HOME"
export PATH="$NDK_BIN:$PATH"

# engine/.cargo/config.toml names <triple>-ar without a path; the NDK ships it
# as llvm-ar.
SHIM_DIR="$ENGINE_DIR/target/android-ar-shim"
mkdir -p "$SHIM_DIR"
ln -sf "$NDK_BIN/llvm-ar" "$SHIM_DIR/$TARGET-ar"
export PATH="$SHIM_DIR:$PATH"

V8_DIR="$ENGINE_DIR/third_party/rusty_v8/$ARCH"
V8_ARCHIVE="$V8_DIR/librusty_v8.a"
V8_BINDING="$V8_DIR/src_binding.rs"
[[ -f "$V8_ARCHIVE" ]] || { err "missing $V8_ARCHIVE"; exit 1; }
[[ -f "$V8_BINDING" ]] || { err "missing $V8_BINDING"; exit 1; }
V8_MANIFEST="$V8_DIR/component-manifest.json"
[[ -f "$V8_MANIFEST" ]] || {
    err "missing verified V8 component manifest $V8_MANIFEST"
    err "rebuild this platform's V8 artifact on the V8 build machine"
    exit 1
}
export RUSTY_V8_ARCHIVE="$V8_ARCHIVE"
export RUSTY_V8_SRC_BINDING_PATH="$V8_BINDING"

MANIFEST_TOOL="${MIGO_ARTIFACT_MANIFEST_TOOL:-}"
if [[ -z "$MANIFEST_TOOL" ]]; then
    MANIFEST_TOOL_TARGET="${MIGO_ARTIFACT_MANIFEST_TARGET_DIR:-$REPO_ROOT/tools/artifact-manifest/target}"
    CARGO_TARGET_DIR="$MANIFEST_TOOL_TARGET" cargo build \
        --manifest-path "$REPO_ROOT/tools/artifact-manifest/Cargo.toml" \
        --locked --release
    MANIFEST_TOOL="$MANIFEST_TOOL_TARGET/release/migo-artifact-manifest"
fi
[[ -x "$MANIFEST_TOOL" ]] || { err "artifact manifest verifier is not executable: $MANIFEST_TOOL"; exit 1; }

info "verifying V8 component bytes before linking"
"$MANIFEST_TOOL" verify-v8-component \
    "$V8_MANIFEST" "$V8_ARCHIVE" "$V8_BINDING" >/dev/null

SNAPSHOT_BIN="$ENGINE_DIR/crates/runtime-v8/snapshots/SNAPSHOT-full-$ARCH.bin"
SNAPSHOT_MANIFEST="$SNAPSHOT_BIN.manifest.json"
[[ -f "$SNAPSHOT_MANIFEST" ]] || {
    err "missing snapshot identity $SNAPSHOT_MANIFEST (regenerate the snapshot)"
    exit 1
}
# Freshness-only CI accepts a Git LFS pointer as an artifact identity. A package
# build cannot: runtime-v8 must embed the real bytes, and a source-JS fallback
# must never be mislabeled as snapshot_policy=embedded in the package manifest.
# shellcheck source=scripts/lib/snapshot-fingerprint.sh
source "$SCRIPT_DIR/lib/snapshot-fingerprint.sh"
snapshot_require_materialized_snapshot "$SNAPSHOT_BIN"
info "verifying the target host/full snapshot before compiling"
bash "$SCRIPT_DIR/check-snapshot-freshness.sh" --product-profile full "$ARCH"

BUILD_METADATA="$ENGINE_DIR/target/$TARGET/release/migo-android-build-metadata.json"
python3 "$SCRIPT_DIR/write-android-build-metadata.py" \
    --repo-root "$REPO_ROOT" \
    --output "$BUILD_METADATA" \
    --ndk-home "$ANDROID_NDK_HOME" \
    --target-triple "$TARGET" \
    --build-recipe scripts/build-android-sdk.sh

info "building capi staticlib (release, $TARGET)"
cargo build -p migo-capi --release --target "$TARGET" --manifest-path "$ENGINE_DIR/Cargo.toml"
STATIC_LIB="$ENGINE_DIR/target/$TARGET/release/libmigo_capi.a"
[[ -f "$STATIC_LIB" ]] || { err "no static library produced"; exit 1; }
info "built: $STATIC_LIB ($(stat -c %s "$STATIC_LIB") bytes)"

info "capturing the link line cargo uses"
CARGO_OUT="$ENGINE_DIR/target/$TARGET/release/migo-android-native-static-libs.txt"
cargo rustc -p migo-capi --release --target "$TARGET" \
    --manifest-path "$ENGINE_DIR/Cargo.toml" -- --print native-static-libs \
    > "$CARGO_OUT" 2>&1 \
    || { err "cargo did not report native-static-libs"; cat "$CARGO_OUT" >&2; exit 1; }
grep -q "native-static-libs:" "$CARGO_OUT" \
    || { err "no native-static-libs note in cargo output"; exit 1; }

info "staging package at $PREFIX"
rm -rf "$PREFIX"
mkdir -p "$PREFIX/include" "$PREFIX/lib"
cp -r "$REPO_ROOT/include/migo" "$PREFIX/include/"
cp "$STATIC_LIB" "$PREFIX/lib/"

python3 "$SCRIPT_DIR/gen-android-package-metadata.py" \
    --prefix "$PREFIX" --version "$VERSION" --arch "$ARCH" --cargo-output "$CARGO_OUT"
python3 "$SCRIPT_DIR/gen-android-package-metadata.py" --manifest \
    --prefix "$PREFIX" --version "$VERSION" --arch "$ARCH" --cargo-output "$CARGO_OUT" \
    --snapshot-bin "$SNAPSHOT_BIN" --snapshot-manifest "$SNAPSHOT_MANIFEST" \
    --v8-component-manifest "$V8_MANIFEST" --build-metadata "$BUILD_METADATA"

PACKAGE_MANIFEST="$PREFIX/share/migo/android-$ABI-manifest.json"
info "verifying the complete staged package tree"
"$MANIFEST_TOOL" verify-android-package "$PACKAGE_MANIFEST" "$PREFIX" >/dev/null

info "package staged:"
find "$PREFIX" -type f | sed "s#^$PREFIX#  <prefix>#" | sort
