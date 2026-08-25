#!/usr/bin/env bash
# Build this consumer against the staged Android package, proving find_package
# resolves and the migo_* entry points link with only the packaged libraries.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../../.." && pwd)"
ABI="${ANDROID_ABI:-arm64-v8a}"
# Which C++ runtime the *consumer* builds against. `c++_shared` is what the SDK
# contract has always proven; the ABI freeze checklist lists the rest of the
# matrix as open, and it is worth closing rather than assuming, because the
# static library carries Chromium's own libc++ inside `librusty_v8.a` and
# `libmigo.so` declares no DT_NEEDED on `libc++_shared.so` at all. A host that
# picks a different setting is therefore not obviously fine or obviously broken.
STL="${ANDROID_STL:-c++_shared}"
# Extra linker flags a consumer might need. Empty by default: whatever a real
# host has to add to make this package link is a property of the package, and
# it belongs in the package's documentation rather than in this script's
# defaults.
EXTRA_LINK="${MIGO_CONSUMER_LINK_FLAGS:-}"
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
    -DANDROID_ABI="$ABI" -DANDROID_PLATFORM=android-26 -DANDROID_STL="$STL" \
    -DCMAKE_SHARED_LINKER_FLAGS="$EXTRA_LINK" \
    -DCMAKE_PREFIX_PATH="$PREFIX" -DCMAKE_FIND_ROOT_PATH="$PREFIX" >/dev/null
cmake --build "$BUILD"
echo "built: $(find "$BUILD" -name 'libconsumer.so')"
