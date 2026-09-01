#!/usr/bin/env bash
# scripts/test-android-so-page-alignment.sh
#
# Assert every shipped Android `.so` is laid out for 16 KB memory pages.
#
# WHY THIS EXISTS (observed, not hypothetical):
# Android 15 introduced devices with a 16 KB page size, and from Android 16 an
# app targeting API 36 must have 16 KB-aligned native libraries to install on
# them. Migo's libraries already satisfy it -- but only because the NDK in use
# defaults to `-z max-page-size=16384`, and nothing in this repository says so.
# The linker flags in `engine/.cargo/config.toml` are extensive and hand-tuned;
# an added `-z max-page-size=4096`, a downgraded NDK, or a different linker
# would drop the alignment back to 4 KB, and the failure would appear as
# "installs fine on every device we own, fails on a device we do not".
#
# WHY IT CHECKS WHAT IT CHECKS:
# It reads the `p_align` of the PT_LOAD program headers, which is the value the
# loader actually uses, rather than a build flag that may or may not have
# reached the linker -- this repository has already been bitten once by
# `RUSTFLAGS` silently replacing the config's flags instead of merging with
# them, so "the flag is in the file" is not evidence the linker saw it.
#
# Usage:
#   scripts/test-android-so-page-alignment.sh [file.so ...]
# With no arguments every `.so` under engine/jniLibs is checked.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TAG='[so-page-align]'
REQUIRED_ALIGN=16384

pass() { echo -e "\033[0;32m$TAG PASS $*\033[0m"; }
fail() { echo -e "\033[0;31m$TAG FAIL $*\033[0m" >&2; failures=$((failures + 1)); }
info() { echo -e "\033[0;36m$TAG $*\033[0m"; }
failures=0

# The NDK comes from the repository's resolver, never from whatever
# `ANDROID_NDK_HOME` happens to hold: reading the alignment with a different
# NDK's tool than the one that produced the library is how a gate ends up
# describing a build nobody shipped. `test-android-ndk-pin-contract.sh`
# enforces that every script does this.
# shellcheck source=scripts/lib/android-ndk.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/android-ndk.sh"

# llvm-readelf from the pinned NDK, resolved by the shared helper -- not the
# GNU one and not whatever `ANDROID_NDK_HOME` happens to hold. The helper
# carries two lessons this gate would otherwise have to relearn: reading an
# artifact with a different NDK's tool than the one that built it describes a
# build nobody shipped, and `find -type f` silently substitutes GNU readelf
# because r23 ships `llvm-readelf` as a symlink.
readelf_bin=""
if ! readelf_bin="$(android_ndk_readelf \
    "$REPO_ROOT/contracts/artifact-manifest/android-v8.lock.json" 2>/dev/null)"; then
    readelf_bin=""
fi
if [[ -z "$readelf_bin" ]]; then
    echo "$TAG llvm-readelf not found (install the pinned NDK)" >&2
    exit 1
fi

libraries=("$@")
if [[ ${#libraries[@]} -eq 0 ]]; then
    while IFS= read -r found; do
        libraries+=("$found")
    done < <(find "$REPO_ROOT/engine/jniLibs" -name '*.so' -type f 2>/dev/null | sort)
fi

if [[ ${#libraries[@]} -eq 0 ]]; then
    echo "$TAG no Android .so found; build one first or pass paths explicitly" >&2
    exit 1
fi

info "every shipped library is laid out for ${REQUIRED_ALIGN}-byte pages"
for so in "${libraries[@]}"; do
    if [[ ! -f "$so" ]]; then
        fail "$so does not exist"
        continue
    fi
    # One p_align per PT_LOAD segment; the smallest is what constrains the
    # loader, so check that rather than the first or the largest.
    smallest=""
    while read -r align; do
        [[ -n "$align" ]] || continue
        value=$((align))
        if [[ -z "$smallest" || "$value" -lt "$smallest" ]]; then
            smallest="$value"
        fi
    done < <("$readelf_bin" -lW "$so" 2>/dev/null | awk '$1=="LOAD"{print $NF}')

    if [[ -z "$smallest" ]]; then
        fail "$(basename "$(dirname "$so")")/$(basename "$so"): no PT_LOAD segments found"
    elif [[ "$smallest" -ge "$REQUIRED_ALIGN" ]]; then
        pass "$(basename "$(dirname "$so")")/$(basename "$so") p_align=$smallest"
    else
        fail "$(basename "$(dirname "$so")")/$(basename "$so") p_align=$smallest, need >= $REQUIRED_ALIGN;"
        fail "  it will not install on a 16 KB-page device running Android 15+"
    fi
done

echo
if (( failures > 0 )); then
    echo -e "\033[0;31m$TAG $failures check(s) failed\033[0m" >&2
    exit 1
fi
echo -e "\033[0;32m$TAG all checks passed\033[0m"
exit 0
