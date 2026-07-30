#!/usr/bin/env bash
# =============================================================================
# Stage the OpenHarmony C SDK package.
# Location: scripts/build-ohos-sdk.sh
#
# Counterpart to scripts/build-android-sdk.sh, and deliberately the same shape:
# an OpenHarmony consumer links a static library through CMake, exactly as an
# NDK consumer does. It therefore ships neither pkg-config nor a versioned
# shared object -- those would be shape without a consumer, not omissions.
#
# Output: dist/migo-ohos-<arch>/
#   include/migo/**          public headers, byte-identical across platforms
#   lib/libmigo_capi.a       the static library
#   lib/cmake/migo/          find_package(migo) support
#   share/migo/*-manifest.json
#   README.md
#
# Usage:
#   scripts/build-ohos-sdk.sh [x86_64|aarch64]   (default: aarch64)
#   scripts/build-ohos-sdk.sh --all
#
# ONE COMMAND. From a clean checkout this builds V8, builds migo-capi, stages
# the package and gates it, in that order, reusing whatever already exists. The
# delivery criterion this project holds every platform to is a single command
# with no human step in the middle, and "run these three and remember two
# environment variables" does not meet it.
#
# Env:
#   MIGO_OHOS_NO_BUILD=1   fail instead of building missing inputs. For CI
#                          lanes that mean to check an existing package rather
#                          than produce one.
#   MIGO_OHOS_MIN_API      declared floor (default 20)
#
# ⚠ profile-slim is used, and that is a requirement rather than a preference:
# the full profile pulls audio -> cpal -> alsa-sys -> pkg-config, and
# OpenHarmony has no ALSA. Its audio surface is OHAudio (libohaudio.so, present
# in the sysroot), which the engine does not consume yet.
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

info() { echo -e "\033[0;36m[ohos-sdk] $*\033[0m"; }
err()  { echo -e "\033[0;31m[ohos-sdk] $*\033[0m" >&2; }
ok()   { echo -e "\033[0;32m[ohos-sdk] $*\033[0m"; }

ARCHES=()
case "${1:-aarch64}" in
    --all)   ARCHES=(x86_64 aarch64) ;;
    x86_64)  ARCHES=(x86_64) ;;
    aarch64) ARCHES=(aarch64) ;;
    *) err "unknown argument: $1"; exit 1 ;;
esac

MIGO_VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$REPO_ROOT/engine/crates/capi/Cargo.toml" | head -1)"
[[ -n "$MIGO_VERSION" ]] || MIGO_VERSION="0.1.0"

# The API level this package declares support for. Derived from Huawei's own
# device-share data rather than from "oldest possible": as of 2026-06 the
# API 12/13/16 share is zero and 6.1.x (API 23/24) is 94.77%, so a floor below
# the current major line buys nothing. It is not set lower than what can
# actually be verified -- see scripts/test-ohos-symbol-floor.sh.
MIN_OHOS_API="${MIGO_OHOS_MIN_API:-20}"

for ARCH in "${ARCHES[@]}"; do
    TRIPLE="$ARCH-unknown-linux-ohos"
    STATIC_LIB="$REPO_ROOT/engine/target/$TRIPLE/release/libmigo_capi.a"
    PREFIX="$REPO_ROOT/dist/migo-ohos-$ARCH"

    # ---- build what is missing ---------------------------------------------
    # One command, no human step in the middle: that is the delivery criterion
    # this project holds every platform to, and three commands with two
    # environment variables to remember is not it. Existing artifacts are
    # reused, so the expensive parts run once.
    V8_DIR="$REPO_ROOT/engine/third_party/rusty_v8/$ARCH-linux-ohos"
    if [[ ! -f "$V8_DIR/librusty_v8.a" ]]; then
        if [[ "${MIGO_OHOS_NO_BUILD:-0}" == "1" ]]; then
            err "missing $V8_DIR/librusty_v8.a and MIGO_OHOS_NO_BUILD=1"
            exit 1
        fi
        info "$ARCH: V8 archive absent, building it (this takes a while)"
        bash "$SCRIPT_DIR/build-v8-ohos.sh" "$ARCH"
    fi
    [[ -f "$V8_DIR/librusty_v8.a" ]] || { err "V8 build produced no archive"; exit 1; }
    [[ -f "$V8_DIR/src_binding.rs" ]] || { err "V8 build produced no binding"; exit 1; }

    if [[ ! -f "$STATIC_LIB" ]]; then
        if [[ "${MIGO_OHOS_NO_BUILD:-0}" == "1" ]]; then
            err "static library not found: ${STATIC_LIB#"$REPO_ROOT"/}"
            err "and MIGO_OHOS_NO_BUILD=1 forbids building it"
            exit 1
        fi
        info "$ARCH: building migo-capi"
        # dev-setup-ohos.sh supplies the compiler pins; without them a machine
        # with an Android NDK on PATH silently compiles the C dependencies with
        # the NDK's clang and bionic headers for a musl target.
        OHOS_ENV="$(bash "$SCRIPT_DIR/dev-setup-ohos.sh" | grep '^export ')"
        (
            eval "$OHOS_ENV"
            cd "$REPO_ROOT/engine"
            # Run from engine/ so rust-toolchain.toml applies: it is resolved
            # from the working directory, not from --manifest-path.
            RUSTY_V8_ARCHIVE="$V8_DIR/librusty_v8.a" \
            RUSTY_V8_SRC_BINDING_PATH="$V8_DIR/src_binding.rs" \
                cargo build -p migo-capi --release \
                    --no-default-features --features profile-slim \
                    --target "$TRIPLE"
        )
    fi

    if [[ ! -f "$STATIC_LIB" ]]; then
        err "build reported success but produced no ${STATIC_LIB#"$REPO_ROOT"/}"
        exit 1
    fi

    # A library that exports no entry points would package and gate cleanly
    # while being useless, so the count is checked before anything is staged.
    ENTRY_POINTS="$(nm --defined-only "$STATIC_LIB" 2>/dev/null | grep -c ' T migo_' || true)"
    if [[ "$ENTRY_POINTS" -eq 0 ]]; then
        err "$STATIC_LIB defines no migo_* entry points"
        exit 1
    fi

    info "$ARCH: $ENTRY_POINTS migo_* entry points"

    rm -rf "$PREFIX"
    mkdir -p "$PREFIX/include" "$PREFIX/lib/cmake/migo" "$PREFIX/share/migo"
    cp -r "$REPO_ROOT/include/migo" "$PREFIX/include/"
    cp "$STATIC_LIB" "$PREFIX/lib/"

    # ---- CMake package ------------------------------------------------------
    # Two things here are load-bearing and were established by linking a real
    # consumer, not by reading documentation:
    #
    # --gc-sections is REQUIRED, not an optimization. skia-bindings contains a
    # translation unit wrapping JPEG/PDF/pathops, features this Skia build has
    # disabled, so that object references symbols which do not exist. Without
    # section garbage collection a consumer link fails with ~20 undefined
    # symbols (SkJpegDecoder, SkPDF, SkOpBuilder, Op, Simplify, AsWinding, ...).
    # The same is true on Linux; it is documented there too.
    #
    # GLESv3 rather than GLESv2: the graphics backend requires ES 3.0.
    cat > "$PREFIX/lib/cmake/migo/migo-config.cmake" <<EOF
# Generated by scripts/build-ohos-sdk.sh -- do not edit.
#
# Consume with the OpenHarmony SDK toolchain:
#   cmake -DCMAKE_TOOLCHAIN_FILE=\$OHOS_NDK_HOME/native/build/cmake/ohos.toolchain.cmake \\
#         -DOHOS_ARCH=$ARCH -DCMAKE_PREFIX_PATH=<this package prefix> ...
cmake_minimum_required(VERSION 3.16)

get_filename_component(MIGO_PREFIX "\${CMAKE_CURRENT_LIST_DIR}/../../.." ABSOLUTE)

set(MIGO_VERSION "$MIGO_VERSION")
set(MIGO_INCLUDE_DIRS "\${MIGO_PREFIX}/include")
set(MIGO_LIBRARY "\${MIGO_PREFIX}/lib/libmigo_capi.a")

add_library(migo::migo STATIC IMPORTED)
set_target_properties(migo::migo PROPERTIES
    IMPORTED_LOCATION "\${MIGO_LIBRARY}"
    INTERFACE_INCLUDE_DIRECTORIES "\${MIGO_INCLUDE_DIRS}"
    INTERFACE_LINK_LIBRARIES "EGL;GLESv3;c++;m;dl;pthread")

# Required, not tuning: skia-bindings carries a translation unit referencing
# JPEG/PDF/pathops symbols for features this build disables. Only section
# garbage collection removes it; without this a consumer link fails with about
# twenty undefined symbols that look like a corrupt library.
set_property(TARGET migo::migo APPEND PROPERTY
    INTERFACE_LINK_OPTIONS "-Wl,--gc-sections")

set(migo_FOUND TRUE)
EOF

    cat > "$PREFIX/lib/cmake/migo/migo-config-version.cmake" <<EOF
# Generated by scripts/build-ohos-sdk.sh -- do not edit.
set(PACKAGE_VERSION "$MIGO_VERSION")
if(PACKAGE_VERSION VERSION_LESS PACKAGE_FIND_VERSION)
    set(PACKAGE_VERSION_COMPATIBLE FALSE)
else()
    set(PACKAGE_VERSION_COMPATIBLE TRUE)
    if(PACKAGE_VERSION VERSION_EQUAL PACKAGE_FIND_VERSION)
        set(PACKAGE_VERSION_EXACT TRUE)
    endif()
endif()
EOF

    # ---- manifest -----------------------------------------------------------
    SDK_PKG="${OHOS_NDK_HOME:-$HOME/ohos-sdk}/native/oh-uni-package.json"
    SDK_API="unknown"; SDK_VER="unknown"
    if [[ -f "$SDK_PKG" ]]; then
        SDK_API="$(sed -n 's/.*"apiVersion"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$SDK_PKG" | head -1)"
        SDK_VER="$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$SDK_PKG" | head -1)"
    fi
    LIB_SHA="$(sha256sum "$PREFIX/lib/libmigo_capi.a" | cut -d' ' -f1)"
    GIT_COMMIT="$(cd "$REPO_ROOT" && git rev-parse --short HEAD 2>/dev/null || echo unknown)"

    cat > "$PREFIX/share/migo/ohos-$ARCH-manifest.json" <<EOF
{
  "schema": "migo/ohos-package/v1",
  "os": "openharmony",
  "arch": "$ARCH",
  "target_triple": "$TRIPLE",
  "product_profile": "slim",
  "min_ohos_api": $MIN_OHOS_API,
  "build_sdk": {
    "version": "$SDK_VER",
    "api_version": "$SDK_API"
  },
  "abi_note": "OpenHarmony userspace is musl. This archive is not interchangeable with an Android (bionic) or a glibc build.",
  "entry_points": $ENTRY_POINTS,
  "link_libraries": ["EGL", "GLESv3", "c++", "m", "dl", "pthread"],
  "required_link_options": ["-Wl,--gc-sections"],
  "capabilities": {
    "attachable_platform_kinds": [],
    "note": "No OpenHarmony surface backend is implemented yet, so migo_query_capabilities reports no attachable kind. Headers describing MIGO_PLATFORM_OPENHARMONY_NATIVE_WINDOW are a compile-time contract only."
  },
  "known_gaps": [
    "audio: OHAudio (libohaudio.so) is not wired up; this package is built with profile-slim",
    "surface attach: not implemented, see capabilities.attachable_platform_kinds"
  ],
  "artifacts": {
    "lib/libmigo_capi.a": "$LIB_SHA"
  },
  "provenance": {
    "git_commit": "$GIT_COMMIT"
  }
}
EOF

    cat > "$PREFIX/README.md" <<EOF
# Migo for OpenHarmony ($ARCH)

Static library plus public headers, consumed through CMake the way an
OpenHarmony native dependency normally is.

\`\`\`cmake
find_package(migo REQUIRED)
target_link_libraries(your_app PRIVATE migo::migo)
\`\`\`

Configure with the SDK toolchain:

\`\`\`sh
cmake -DCMAKE_TOOLCHAIN_FILE=\$OHOS_NDK_HOME/native/build/cmake/ohos.toolchain.cmake \\
      -DOHOS_ARCH=$ARCH \\
      -DCMAKE_PREFIX_PATH=<this directory> ...
\`\`\`

## What this package does not do yet

\`migo_query_capabilities\` reports **no attachable platform kind**: the
OpenHarmony surface backend is not implemented. The headers declaring
\`MIGO_PLATFORM_OPENHARMONY_NATIVE_WINDOW\` are a compile-time contract, and
this package deliberately does not claim otherwise -- a published SDK that
loads, resolves every entry point and can attach nothing is a failure mode this
project has already shipped once on another platform.

Audio is likewise absent: the package is built with \`profile-slim\` because
the full profile requires ALSA, which OpenHarmony does not have. Wiring the
engine to OHAudio is separate work.

Declared minimum OpenHarmony API: **$MIN_OHOS_API**.
See \`share/migo/ohos-$ARCH-manifest.json\` for the complete record.
EOF

    ok "staged ${PREFIX#"$REPO_ROOT"/} ($(du -sh "$PREFIX" | cut -f1))"
done

# ---- gate its own output ----------------------------------------------------
# The build script runs the contract itself rather than leaving it to CI. There
# is no OpenHarmony runner, so a gate that only runs in CI would never run at
# all -- the same reasoning the Windows SDK script already follows.
info "running the package contract"
for ARCH in "${ARCHES[@]}"; do
    bash "$SCRIPT_DIR/test-ohos-sdk-contract.sh" "$REPO_ROOT/dist/migo-ohos-$ARCH"
done
