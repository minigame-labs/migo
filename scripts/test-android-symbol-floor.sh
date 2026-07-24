#!/usr/bin/env bash
# =============================================================================
# Android minimum-API symbol floor gate.
#
# Every symbol libmigo.so imports from the platform must exist at the minSdk
# Migo claims (API 26). If it imports one that the platform only added later,
# the whole library fails to `dlopen` on a floor device -- the runtime does not
# start at all, and nothing above it runs. That failure is invisible on a newer
# device or emulator, so it cannot be a manual check; it has to be a gate.
#
# This caught exactly that on 2026-07-24: `libmigo.so` imported
# `android_get_device_api_level`, which is an exported libc symbol only from
# API 29 on (at API 21..=28 it is a `static inline` in <android/api-level.h>).
# On the API-26 emulator the load failed with "cannot locate symbol
# android_get_device_api_level", and neither the API-29 emulator nor the API-31
# phone showed it.
#
# The authority for "exists at API 26" is the NDK's own versioned sysroot stub
# libraries under .../sysroot/usr/lib/<triple>/26/. A symbol is allowed iff some
# API-26 stub library exports it (or libmigo.so defines it itself). This is not
# a hand-maintained allowlist: it is what the NDK guarantees is present at 26.
#
# Usage:
#   scripts/test-android-symbol-floor.sh <path/to/libmigo.so> [more.so ...]
#
# With no argument it checks every jniLibs/<abi>/libmigo.so a local AAR build
# staged. Fails closed: a missing NDK, missing stubs, or an unreadable library
# is an error, not a skip.
# =============================================================================
set -euo pipefail

MIN_API="${MIGO_MIN_API:-26}"
ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Ndk}"

err() { echo -e "\033[0;31m[floor] $*\033[0m" >&2; }
ok()  { echo -e "\033[0;32m[floor] $*\033[0m"; }
info(){ echo -e "\033[0;36m[floor] $*\033[0m"; }

[[ -d "$ANDROID_NDK_HOME" ]] || { err "ANDROID_NDK_HOME not found: $ANDROID_NDK_HOME"; exit 1; }
NDK_BIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
NM="$NDK_BIN/llvm-nm"
[[ -x "$NM" ]] || { err "llvm-nm not found at $NM"; exit 1; }
SYSROOT_LIB="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib"

# Map an ABI to its NDK triple. The triple names the stub directory.
triple_for() {
    case "$1" in
        arm64-v8a|aarch64) echo aarch64-linux-android ;;
        x86_64)            echo x86_64-linux-android ;;
        armeabi-v7a)       echo arm-linux-androideabi ;;
        x86)               echo i686-linux-android ;;
        *) return 1 ;;
    esac
}

# Detect the ABI from the path (…/jniLibs/<abi>/libmigo.so) or the ELF header.
abi_of() {
    local so="$1" abi
    abi="$(basename "$(dirname "$so")")"
    if triple_for "$abi" >/dev/null 2>&1; then echo "$abi"; return; fi
    case "$("$NM" --help >/dev/null 2>&1; file -b "$so" 2>/dev/null)" in
        *x86-64*) echo x86_64 ;;
        *aarch64*|*ARM\ aarch64*) echo arm64-v8a ;;
        *) return 1 ;;
    esac
}

check_one() {
    local so="$1"
    [[ -f "$so" ]] || { err "not a file: $so"; return 1; }

    local abi triple stubdir
    abi="$(abi_of "$so")" || { err "cannot determine ABI for $so"; return 1; }
    triple="$(triple_for "$abi")" || { err "unsupported ABI '$abi' for $so"; return 1; }
    stubdir="$SYSROOT_LIB/$triple/$MIN_API"
    [[ -d "$stubdir" ]] || { err "no NDK stub libraries for API $MIN_API at $stubdir"; return 1; }

    # Symbols the platform guarantees at MIN_API: the union of every stub .so's
    # dynamic exports. Built fresh from the NDK, never hand-maintained.
    local allowed imported internal undefined violations
    allowed="$(for lib in "$stubdir"/*.so; do
        "$NM" -D --defined-only "$lib" 2>/dev/null | awk '$2 ~ /[A-Za-z]/ {print $NF}'
    done | LC_ALL=C sort -u)"

    # What libmigo.so imports, minus what it defines itself (symbols can appear
    # as both when different objects reference and provide them).
    #
    # Only STRONG undefined symbols (nm type `U`) matter. A WEAK undefined
    # symbol (`w`) that the loader cannot resolve becomes NULL rather than
    # failing the load, which is exactly how libc lets a binary reference a
    # newer entry point and fall back when it is absent -- `getrandom` (API 28)
    # and `copy_file_range` (API 29) appear this way and are safe on API 26.
    # A strong `U` is the one that aborts `dlopen`, so the gate must not flag
    # weak imports or it fails a library that actually loads.
    undefined="$("$NM" -D --undefined-only "$so" 2>/dev/null | awk '$1 == "U" {print $2}' | grep -v '^$' | LC_ALL=C sort -u)"
    internal="$("$NM" -D --defined-only "$so" 2>/dev/null | awk '$2 ~ /[A-Za-z]/ {print $NF}' | LC_ALL=C sort -u)"
    imported="$(LC_ALL=C comm -23 <(printf '%s\n' "$undefined") <(printf '%s\n' "$internal"))"

    # Anything imported that neither the API-MIN stubs nor libmigo itself provide.
    violations="$(LC_ALL=C comm -23 <(printf '%s\n' "$imported") <(printf '%s\n' "$allowed"))"

    if [[ -n "$violations" ]]; then
        err "$so ($abi): imports symbols not present at API $MIN_API:"
        printf '%s\n' "$violations" | sed 's/^/         /' >&2
        return 1
    fi
    ok "$so ($abi): all imports resolve at API $MIN_API"
    return 0
}

TARGETS=("$@")
if [[ ${#TARGETS[@]} -eq 0 ]]; then
    REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
    mapfile -t TARGETS < <(find "$REPO_ROOT/engine/jniLibs" -name libmigo.so 2>/dev/null)
    [[ ${#TARGETS[@]} -gt 0 ]] || { err "no libmigo.so given and none staged under engine/jniLibs"; exit 1; }
    info "checking staged libraries: ${TARGETS[*]}"
fi

rc=0
for so in "${TARGETS[@]}"; do
    check_one "$so" || rc=1
done
exit "$rc"
