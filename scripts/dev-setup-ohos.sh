#!/usr/bin/env bash
# scripts/dev-setup-ohos.sh
#
# Make an OpenHarmony native SDK usable for cross-compiling migo to
# *-unknown-linux-ohos. Counterpart to scripts/dev-setup-skia.sh.
#
# This script does NOT download the SDK. OpenHarmony's release archives are
# version- and mirror-specific, and a URL baked in here would rot silently.
# It locates an already-installed SDK, asserts the parts migo actually needs,
# reports the two version numbers that decide downstream behaviour, and prints
# the exports the build scripts consume.
#
# To install one (verified 2026-07-30, ~3.2 GB, Linux and Windows share a
# single archive):
#
#   BASE=https://repo.huaweicloud.com/openharmony/os/5.1.0-Release
#   curl -fsSL -O "$BASE/ohos-sdk-windows_linux-public.tar.gz.sha256"
#   curl -fL --retry 3 -C - -O "$BASE/ohos-sdk-windows_linux-public.tar.gz"
#   # NOTE: that .sha256 holds a bare hash with no filename, so `sha256sum -c`
#   # reports "no properly formatted checksum lines found" -- which reads like
#   # a corrupt download but is only a format mismatch. Compare by hand:
#   #   [[ "$(tr -d '[:space:]' < *.sha256)" == "$(sha256sum *.tar.gz | cut -d' ' -f1)" ]]
#   tar -xzf ohos-sdk-windows_linux-public.tar.gz ohos-sdk/linux/native-linux-x64-*.zip
#   unzip -q ohos-sdk/linux/native-linux-x64-*.zip -d "$HOME/ohos-sdk"
#
# The tar layout is NOT stable across releases -- setup-ohos-sdk's installer
# carries per-version strip levels -- so locate the component with a glob or
# `find` rather than assuming a depth.
#
# Usage:
#   scripts/dev-setup-ohos.sh            # locate, assert, print exports
#   scripts/dev-setup-ohos.sh --check    # assert only; exit 1 if unusable
#
# Env:
#   OHOS_NDK_HOME  parent of the `native/` directory. If unset, the well-known
#                  locations below are probed in order.
set -euo pipefail

info() { echo -e "\033[0;36m[ohos-setup] $*\033[0m"; }
err()  { echo -e "\033[0;31m[ohos-setup] $*\033[0m" >&2; }

# Written as a full `if`, not `[[ ... ]] && CHECK_ONLY=1`. Under `set -e` a
# bare `[[ ... ]] && x` statement whose condition is false returns 1 and kills
# the script, so the no-argument path would exit before doing anything.
CHECK_ONLY=0
if [[ "${1:-}" == "--check" ]]; then
    CHECK_ONLY=1
fi

# ---- locate -----------------------------------------------------------------
CANDIDATES=(
    "${OHOS_NDK_HOME:-}"
    "$HOME/ohos-sdk"
    "$HOME/.ohos-sdk"
    "/opt/ohos-sdk"
    "/usr/local/ohos-sdk"
)

NDK_HOME=""
for c in "${CANDIDATES[@]}"; do
    [[ -n "$c" && -d "$c/native" ]] || continue
    NDK_HOME="$c"
    break
done

if [[ -z "$NDK_HOME" ]]; then
    err "no OpenHarmony SDK found."
    err "Install one (see the header of this script), then set OHOS_NDK_HOME"
    err "to the directory containing native/."
    err "Probed: ${CANDIDATES[*]}"
    exit 1
fi

NATIVE="$NDK_HOME/native"
info "SDK: $NDK_HOME"

# ---- assert the parts migo needs -------------------------------------------
# Each of these is consumed by a specific downstream step; a missing one fails
# far away from here if it is not caught now.
REQUIRED=(
    "$NATIVE/llvm/bin/clang"                 # cc-rs / skia-bindings compiler
    "$NATIVE/sysroot/usr/include/stdio.h"    # musl libc headers
    "$NATIVE/sysroot/usr/lib"                # link path root
)

MISSING=0
for f in "${REQUIRED[@]}"; do
    if [[ -e "$f" ]]; then
        info "  ok   ${f#"$NDK_HOME"/}"
    else
        err  "  MISS ${f#"$NDK_HOME"/}"
        MISSING=1
    fi
done
if [[ $MISSING -ne 0 ]]; then
    err "SDK at $NDK_HOME is incomplete"
    exit 1
fi

# ---- report the two versions that decide downstream behaviour ---------------
# Reported rather than left for someone to look up, because each one silently
# changes what happens several build steps later.
API_VERSION="unknown"
SDK_VERSION="unknown"
if [[ -f "$NATIVE/oh-uni-package.json" ]]; then
    API_VERSION="$(sed -n 's/.*"apiVersion"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
        "$NATIVE/oh-uni-package.json" | head -1)"
    SDK_VERSION="$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
        "$NATIVE/oh-uni-package.json" | head -1)"
fi
info "SDK version: ${SDK_VERSION:-unknown}  (API ${API_VERSION:-unknown})"

CLANG_VER="$("$NATIVE/llvm/bin/clang" --version | head -1)"
info "clang: $CLANG_VER"

# The SDK's clang is old relative to what V8's vendored libc++ headers expect.
# That does not block compiling migo's own code, but it does decide how V8's
# bindgen step has to be driven -- and bindgen's failure mode is silent: the
# headers get laid out wrong and the only symptom is a static assertion in the
# generated binding failing with a subtraction overflow, which reads like
# broken V8 source rather than a mis-resolved libclang.
CLANG_MAJOR="$(printf '%s' "$CLANG_VER" | sed -n 's/.*clang version \([0-9]\+\).*/\1/p')"
if [[ -n "$CLANG_MAJOR" && "$CLANG_MAJOR" -lt 20 ]]; then
    info "  note: clang $CLANG_MAJOR is older than V8's vendored libc++ headers"
    info "        expect; the V8 build will likely need V8_PREBUILT_BINDING."
fi

# ---- report the target triples the sysroot actually carries -----------------
if [[ -d "$NATIVE/sysroot/usr/lib" ]]; then
    TRIPLES="$(find "$NATIVE/sysroot/usr/lib" -maxdepth 1 -mindepth 1 -type d \
        -name '*-linux-ohos' -printf '%f ' 2>/dev/null || true)"
    info "sysroot targets: ${TRIPLES:-none found}"
fi

# ---- assert the target-prefixed drivers cargo's config.toml names ----------
# The SDK ships one clang driver per target, named exactly after the Rust
# target triple, and each one already knows its own sysroot (verified with
# `-print-search-dirs`: it resolves ../../sysroot/usr/lib without --sysroot).
# So .cargo/config.toml can name the driver directly, the same way the Android
# entries name the NDK's own wrappers -- no hand-rolled wrapper is needed, and
# nothing machine-specific has to be committed.
DRIVERS_OK=1
for triple in x86_64-unknown-linux-ohos aarch64-unknown-linux-ohos; do
    if [[ -x "$NATIVE/llvm/bin/$triple-clang" ]]; then
        info "  ok   llvm/bin/$triple-clang"
    else
        err  "  MISS llvm/bin/$triple-clang"
        DRIVERS_OK=0
    fi
done
if [[ $DRIVERS_OK -ne 1 ]]; then
    err "the SDK does not ship the target-prefixed clang drivers that"
    err "engine/.cargo/config.toml names as linkers"
    exit 1
fi

if [[ $CHECK_ONLY -eq 1 ]]; then
    exit 0
fi

cat <<EOF

# Consume these in the current shell:
export OHOS_NDK_HOME="$NDK_HOME"
export OHOS_SDK_NATIVE="$NATIVE"
export PATH="$NATIVE/llvm/bin:\$PATH"

# ---- pin every C/C++ compiler resolution at the SDK's own clang -------------
# Without these, a machine with an Android NDK on PATH silently builds the C
# dependencies (zstd, sqlite3, ...) with the NDK's clang 12 and its bionic
# headers, for a musl target. It COMPILES -- cc-rs passes --target so the
# object files carry the right triple -- and the mismatch only shows up at
# runtime as struct layouts that disagree. Verified 2026-07-30: the first
# ohos build here produced zstd_preSplit.o stamped
# "Android (8481493 ...) clang version 12.0.9".
#
# cc-rs resolves CC_<target> before plain CC, which is the same lever
# engine/.cargo/config.toml already uses for the Windows MSVC target, so these
# win over an ambient CC without disturbing any other target.
export CC_x86_64_unknown_linux_ohos="$NATIVE/llvm/bin/x86_64-unknown-linux-ohos-clang"
export CXX_x86_64_unknown_linux_ohos="$NATIVE/llvm/bin/x86_64-unknown-linux-ohos-clang++"
export AR_x86_64_unknown_linux_ohos="$NATIVE/llvm/bin/llvm-ar"
export CC_aarch64_unknown_linux_ohos="$NATIVE/llvm/bin/aarch64-unknown-linux-ohos-clang"
export CXX_aarch64_unknown_linux_ohos="$NATIVE/llvm/bin/aarch64-unknown-linux-ohos-clang++"
export AR_aarch64_unknown_linux_ohos="$NATIVE/llvm/bin/llvm-ar"

# skia-bindings does NOT use cc-rs's target prefixes. It reads CLANGCC, then
# plain CC, then the literal "clang" -- so the target-scoped names above are
# invisible to it and it needs its own pin. CLANGCC exists precisely for
# SDK-supplied toolchains, which is what this is.
export CLANGCC="$NATIVE/llvm/bin/clang"
export CLANGCXX="$NATIVE/llvm/bin/clang++"
EOF
