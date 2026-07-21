#!/usr/bin/env bash
# Build this consumer against the staged Android package, proving find_package
# resolves and the migo_* entry points link with only the packaged libraries.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../../.." && pwd)"
ABI="${ANDROID_ABI:-arm64-v8a}"
PREFIX="${MIGO_ANDROID_PREFIX:-$REPO_ROOT/dist/migo-android-$ABI}"
ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Ndk}"

[[ -f "$PREFIX/lib/cmake/migo/migo-config.cmake" ]] || {
    echo "no package at $PREFIX; run scripts/build-android-sdk.sh --arch ${ABI/arm64-v8a/aarch64}" >&2
    exit 1
}

BUILD="$HERE/build"
rm -rf "$BUILD"
# CMAKE_FIND_ROOT_PATH, not only CMAKE_PREFIX_PATH: the NDK toolchain sets
# CMAKE_FIND_ROOT_PATH_MODE_PACKAGE=ONLY for cross-compilation, which confines
# find_package to the find-root. A package staged outside the NDK sysroot is
# found only when its prefix is a find-root -- the documented way an NDK build
# consumes a host-staged package.
cmake -S "$HERE" -B "$BUILD" \
    -DCMAKE_TOOLCHAIN_FILE="$ANDROID_NDK_HOME/build/cmake/android.toolchain.cmake" \
    -DANDROID_ABI="$ABI" -DANDROID_PLATFORM=android-26 -DANDROID_STL=c++_shared \
    -DCMAKE_PREFIX_PATH="$PREFIX" -DCMAKE_FIND_ROOT_PATH="$PREFIX" >/dev/null
cmake --build "$BUILD"
echo "built: $(find "$BUILD" -name 'libconsumer.so')"
