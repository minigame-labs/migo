#!/usr/bin/env bash
# Every .so an Android AAR ships must be one the engine actually loads.
#
# The drift this gate exists to catch was found in the published v0.9.3 AAR, and
# it had been there long enough that nobody remembered it was wrong.
#
# `scripts/build-android-so.sh` copied `libc++_shared.so` next to `libmigo.so`
# on every build, with a comment saying cpal/oboe needed the shared STL. That
# had stopped being true. `librusty_v8.a` carries Chromium's own libc++
# statically, the link resolves the C++ runtime from it, and the produced
# `libmigo.so` declares no DT_NEEDED entry for `libc++_shared.so` -- on arm64
# full, on arm64 slim, and on the x86_64 binary inside the shipped AAR. Its only
# two undefined C++-ish symbols, `__cxa_atexit` and `__cxa_finalize`, are
# bionic's. Removing the file and running a game on a Mate 30 Pro changed
# nothing, audio included -- the subsystem the comment named.
#
# So the AAR shipped ~1 MB per ABI that no loader ever opened, ~2 MB in the
# dual-ABI release. Worse than the bytes: an SDK that ships `libc++_shared.so`
# is picking a libc++ version for its host. When two AARs both provide one,
# Gradle keeps a single copy, and the library that loses gets an ABI it was not
# built against -- a crash in someone else's code, caused by us.
#
# The rule is therefore not "do not ship libc++_shared.so". It is that a
# payload .so must be reachable: either it is the engine, or the engine (or
# another shipped .so) names it in DT_NEEDED. That keeps working if a future
# dependency genuinely needs the shared STL, and it fails the day a copy step
# starts shipping something nothing loads.
#
# Usage: scripts/test-android-native-deps-contract.sh [aar ...]
#   With no arguments, checks every AAR in platforms/android/dist/.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

fail() { echo "FAIL: $*" >&2; exit 1; }

# `readelf` from the NDK if we can find it, otherwise whatever is on PATH.
resolve_readelf() {
    local ndk="${ANDROID_NDK_HOME:-}"
    if [[ -n "$ndk" ]]; then
        local candidate
        candidate="$(find "$ndk/toolchains/llvm/prebuilt" -name llvm-readelf -type f 2>/dev/null | head -1)"
        [[ -n "$candidate" ]] && { echo "$candidate"; return 0; }
    fi
    local found
    found="$(find "$HOME/Android" -name llvm-readelf -type f 2>/dev/null | head -1)"
    [[ -n "$found" ]] && { echo "$found"; return 0; }
    command -v llvm-readelf 2>/dev/null && return 0
    command -v readelf 2>/dev/null && return 0
    return 1
}

READELF="$(resolve_readelf)" || fail "no readelf found (set ANDROID_NDK_HOME, or install binutils)"

aars=("$@")
if [[ ${#aars[@]} -eq 0 ]]; then
    while IFS= read -r f; do aars+=("$f"); done < <(
        find "$REPO_ROOT/platforms/android/dist" -maxdepth 1 -name '*.aar' 2>/dev/null | sort
    )
fi
[[ ${#aars[@]} -gt 0 ]] || { echo "SKIP: no AAR to check"; exit 0; }

checked=0
for aar in "${aars[@]}"; do
    [[ -f "$aar" ]] || fail "not a file: $aar"
    name="$(basename "$aar")"

    work="$(mktemp -d)"
    trap 'rm -rf "$work"' EXIT

    # An AAR with no jni/ at all is the derived -nojni artifact; it has nothing
    # to check and its own gate covers what it must not contain.
    mapfile -t sos < <(unzip -Z1 "$aar" 'jni/*/*.so' 2>/dev/null | sort || true)
    if [[ ${#sos[@]} -eq 0 ]]; then
        echo "  $name: no payload .so (derived artifact); skipped"
        continue
    fi

    unzip -qo "$aar" 'jni/*/*.so' -d "$work"

    # Per ABI: collect what is shipped, and what the shipped files ask for.
    mapfile -t abis < <(printf '%s\n' "${sos[@]}" | cut -d/ -f2 | sort -u)
    for abi in "${abis[@]}"; do
        shipped=()
        while IFS= read -r p; do shipped+=("$(basename "$p")"); done < <(
            printf '%s\n' "${sos[@]}" | grep "^jni/$abi/"
        )

        needed=""
        for base in "${shipped[@]}"; do
            needed+=$("$READELF" --dynamic "$work/jni/$abi/$base" 2>/dev/null \
                | sed -n 's/.*NEEDED.*Shared library: \[\(.*\)\].*/\1/p')$'\n'
        done

        for base in "${shipped[@]}"; do
            # The engine itself is the entry point; it is loaded by name.
            [[ "$base" == "libmigo.so" ]] && continue
            if ! grep -qxF "$base" <<<"$needed"; then
                fail "$name [$abi] ships $base, but no shipped .so declares it in DT_NEEDED.
      Nothing will ever load it, and an SDK that ships a runtime library picks
      that library's version for its host. Either drop it from the packaging
      step, or make the engine actually depend on it."
            fi
        done
        echo "  $name [$abi]: ${#shipped[@]} .so, all reachable"
        checked=$((checked + 1))
    done

    rm -rf "$work"; trap - EXIT
done

echo "PASS: every shipped .so is reachable ($checked ABI payloads checked)"
