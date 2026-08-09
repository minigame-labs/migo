#!/usr/bin/env bash
# =============================================================================
# OpenHarmony minimum-API symbol floor gate.
#
# Every platform symbol migo imports must exist on the oldest OpenHarmony
# release it claims to support. Importing a newer one makes the whole library
# fail to load on a floor device -- nothing above it runs -- and that failure is
# invisible on a newer device or emulator.
#
# WHY THIS MATTERS MORE ON OPENHARMONY THAN ON ANDROID:
# The Android NDK annotates each declaration with __INTRODUCED_IN(api), so
# misusing a newer symbol is at least a compile-time signal, and it ships
# per-API stub libraries (usr/lib/<triple>/26/...) that make "exists at API 26"
# a checkable fact. OpenHarmony has NEITHER. Its version information is a
# `@since` line in a doc comment -- e.g. native_window/graphic_error_code.h
# says `@since 12` -- which the compiler never reads, and its sysroot carries
# exactly one unversioned copy of each library. So on OpenHarmony there is no
# compile-time signal and no per-API stub: this gate is the only check that
# exists.
#
# The scale of the risk is measurable: between API 18 and API 23, libace_ndk
# alone grew from 723 to 1201 exported symbols and the sysroot gained 15
# libraries. Building against API 23 while claiming to support API 20 makes
# picking up one of those 478 additions easy and silent.
#
# THE AUTHORITY THIS USES, AND HOW IT IS DELIBERATELY CONSERVATIVE:
# Lacking per-API stubs, the only objective version evidence available is a
# sysroot from an older SDK. A symbol present in the older sysroot certainly
# exists at every API level in between, so treating the older sysroot as the
# floor is sound -- and stricter than the declared floor, which is the safe
# direction. It is NOT precise: a symbol added at, say, API 19 would be
# reported here even though the declared floor of API 20 permits it. When that
# happens, the fix is to install the SDK matching the declared floor and point
# MIGO_OHOS_FLOOR_SYSROOT at it -- not to add the symbol to an exception list,
# which would recreate the hand-maintained allowlist this avoids.
#
# Usage:
#   scripts/test-ohos-symbol-floor.sh <artifact> [more...]
#     artifact: a .a or .so built for an ohos target
#
# Env:
#   MIGO_OHOS_FLOOR_SYSROOT   sysroot representing the claimed floor
#                             (default: ~/ohos-sdk/native/sysroot, API 18)
#   MIGO_OHOS_TRIPLE          sysroot lib subdirectory
#                             (default: x86_64-linux-ohos)
#
# Fails closed: a missing sysroot, an unreadable artifact, or zero platform
# symbols found is an error, not a skip.
# =============================================================================
set -euo pipefail

FLOOR_SYSROOT="${MIGO_OHOS_FLOOR_SYSROOT:-$HOME/ohos-sdk/native/sysroot}"
TRIPLE="${MIGO_OHOS_TRIPLE:-x86_64-linux-ohos}"

err()  { echo -e "\033[0;31m[ohos-floor] $*\033[0m" >&2; }
ok()   { echo -e "\033[0;32m[ohos-floor] $*\033[0m"; }
info() { echo -e "\033[0;36m[ohos-floor] $*\033[0m"; }

if [[ $# -lt 1 ]]; then
    err "usage: $0 <artifact> [more...]"
    exit 1
fi

FLOOR_LIB="$FLOOR_SYSROOT/usr/lib/$TRIPLE"
if [[ ! -d "$FLOOR_LIB" ]]; then
    err "floor sysroot lib dir not found: $FLOOR_LIB"
    err "set MIGO_OHOS_FLOOR_SYSROOT to an OpenHarmony SDK's native/sysroot"
    exit 1
fi

# Report which SDK is acting as the floor. Leaving this implicit is how a gate
# silently starts measuring against the wrong baseline.
FLOOR_PKG="$(dirname "$FLOOR_SYSROOT")/oh-uni-package.json"
FLOOR_API="unknown"
if [[ -f "$FLOOR_PKG" ]]; then
    FLOOR_API="$(sed -n 's/.*"apiVersion"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$FLOOR_PKG" | head -1)"
fi
info "floor sysroot: $FLOOR_SYSROOT (API ${FLOOR_API:-unknown})"
info "triple:        $TRIPLE"

# The triple selects which sysroot the artifact is measured against, and it
# defaults rather than being derived, so pointing this gate at an aarch64
# artifact without setting it measures against x86_64 libraries. Most symbol
# names are identical across architectures, so that mistake passes -- a gate
# reporting a result for something it did not examine. Check the artifact's own
# machine instead of trusting the caller.
case "$TRIPLE" in
    x86_64-linux-ohos)  WANT_MACHINE="X86-64" ;;
    aarch64-linux-ohos) WANT_MACHINE="AArch64" ;;
    *) err "unknown triple: $TRIPLE"; exit 1 ;;
esac
for artifact in "$@"; do
    [[ -f "$artifact" ]] || { err "artifact not found: $artifact"; exit 1; }
    # One member is enough: an archive mixing architectures would not have
    # linked. `readelf -h` on an archive prints a header per member -- thousands
    # of them here -- so the reader stops at the first Machine line. Stopping
    # early SIGPIPEs readelf, which under `set -o pipefail` fails the pipeline
    # and, under `set -e`, killed this script with status 141 before it could
    # report anything. The `|| true` is what makes an intentional early exit
    # distinguishable from a real failure.
    GOT_MACHINE="$(readelf -h "$artifact" 2>/dev/null \
        | awk '/Machine:/ { sub(/^[[:space:]]*Machine:[[:space:]]*/, ""); print; exit }' \
        || true)"
    if [[ -z "$GOT_MACHINE" ]]; then
        err "cannot read the machine type of $artifact; refusing to report a"
        err "floor result for an artifact this gate could not identify"
        exit 1
    fi
    if [[ "$GOT_MACHINE" != *"$WANT_MACHINE"* ]]; then
        err "$artifact is $GOT_MACHINE but the floor sysroot selected is $TRIPLE"
        err "set MIGO_OHOS_TRIPLE to match the artifact"
        exit 1
    fi
done

# ---- 1. index every symbol the floor platform exports -----------------------
FLOOR_SYMS="$(mktemp)"
trap 'rm -f "$FLOOR_SYMS" "${ARTIFACT_UNDEF:-}"' EXIT

# Counted separately, not inside the loop: piping the loop into `sort` runs it
# in a subshell, so any counter incremented there is lost when it exits. An
# earlier revision did exactly that and reported "0 libraries, 8200 symbols" --
# self-contradictory, and caught only because the anti-vacuity check below
# reads both numbers.
LIB_COUNT="$(find "$FLOOR_LIB" -maxdepth 1 -name '*.so' -type f | wc -l)"
while IFS= read -r so; do
    # Dynamic exports only: that is what a consumer can actually bind to.
    nm -D --defined-only "$so" 2>/dev/null | awk '{print $NF}'
done < <(find "$FLOOR_LIB" -maxdepth 1 -name '*.so' -type f) | sort -u > "$FLOOR_SYMS"

FLOOR_COUNT="$(wc -l < "$FLOOR_SYMS")"
if [[ "$LIB_COUNT" -eq 0 || "$FLOOR_COUNT" -eq 0 ]]; then
    err "indexed $LIB_COUNT libraries and $FLOOR_COUNT symbols from $FLOOR_LIB"
    err "an empty index would let every symbol pass; refusing to report success"
    exit 1
fi
info "floor exports: $FLOOR_COUNT symbols across $LIB_COUNT libraries"

# ---- 2. check each artifact -------------------------------------------------
TOTAL_VIOLATIONS=0
for artifact in "$@"; do
    if [[ ! -f "$artifact" ]]; then
        err "artifact not found: $artifact"
        exit 1
    fi

    ARTIFACT_UNDEF="$(mktemp)"
    ARTIFACT_RAW_UNDEF="$(mktemp)"
    ARTIFACT_DEF="$(mktemp)"
    case "$artifact" in
        *.so|*.so.*)
            nm -D --undefined-only "$artifact" 2>/dev/null | awk '{print $NF}' | sort -u > "$ARTIFACT_RAW_UNDEF"
            nm -D --defined-only  "$artifact" 2>/dev/null | awk '{print $NF}' | sort -u > "$ARTIFACT_DEF"
            ;;
        *)
            nm --undefined-only "$artifact" 2>/dev/null | awk '{print $NF}' | sort -u > "$ARTIFACT_RAW_UNDEF"
            nm --defined-only  "$artifact" 2>/dev/null | awk '{print $NF}' | sort -u > "$ARTIFACT_DEF"
            ;;
    esac

    # Subtract what the artifact satisfies internally. In a static archive
    # every member's cross-references show up as undefined, so the raw list
    # includes symbols another member defines -- the linker never looks for
    # those on the platform.
    #
    # This is not a refinement, it is a correctness fix: the first run of this
    # gate reported seven ICU symbols (u_charAge, ubrk_setUText, ...) as
    # requiring a newer OpenHarmony. All seven are defined inside the archive
    # itself, which bundles its own ICU across 469 object files. A gate that
    # reports symbols the linker will never resolve externally produces noise,
    # and noise is how real findings get ignored.
    comm -23 "$ARTIFACT_RAW_UNDEF" "$ARTIFACT_DEF" > "$ARTIFACT_UNDEF"
    SELF_SATISFIED="$(comm -12 "$ARTIFACT_RAW_UNDEF" "$ARTIFACT_DEF" | wc -l)"
    rm -f "$ARTIFACT_RAW_UNDEF" "$ARTIFACT_DEF"

    UNDEF_COUNT="$(wc -l < "$ARTIFACT_UNDEF")"
    if [[ "$UNDEF_COUNT" -eq 0 ]]; then
        err "$artifact has no undefined symbols at all"
        err "that is not a pass -- it means nm read nothing usable from it"
        exit 1
    fi

    # A symbol is "platform" iff the floor sysroot exports it. Everything else
    # is either internal, libc++, or resolved within the artifact -- this gate
    # has nothing to say about those, and guessing would produce noise that
    # gets suppressed and takes the real findings with it.
    PLATFORM_HITS="$(comm -12 "$ARTIFACT_UNDEF" "$FLOOR_SYMS" | wc -l)"

    info "$(basename "$artifact"): $UNDEF_COUNT externally undefined ($SELF_SATISFIED satisfied internally), $PLATFORM_HITS resolved by the floor platform"

    # Violations: imported, absent from the floor, but present in a newer SDK.
    # The newer-SDK half is what distinguishes "added after the floor" from
    # "not a platform symbol at all"; without it every libc++ symbol would be
    # reported.
    if [[ -n "${MIGO_OHOS_NEWER_SYSROOT:-}" ]]; then
        NEWER_LIB="$MIGO_OHOS_NEWER_SYSROOT/usr/lib/$TRIPLE"
        if [[ -d "$NEWER_LIB" ]]; then
            NEWER_SYMS="$(mktemp)"
            while IFS= read -r so; do
                nm -D --defined-only "$so" 2>/dev/null | awk '{print $NF}'
            done < <(find "$NEWER_LIB" -maxdepth 1 -name '*.so' -type f) | sort -u > "$NEWER_SYMS"

            VIOLATIONS="$(comm -12 "$ARTIFACT_UNDEF" \
                <(comm -13 "$FLOOR_SYMS" "$NEWER_SYMS") | head -40)"
            VCOUNT="$(comm -12 "$ARTIFACT_UNDEF" \
                <(comm -13 "$FLOOR_SYMS" "$NEWER_SYMS") | wc -l)"
            ADDED="$(comm -13 "$FLOOR_SYMS" "$NEWER_SYMS" | wc -l)"
            NEWER_TOTAL="$(wc -l < "$NEWER_SYMS")"
            rm -f "$NEWER_SYMS"

            # An empty or floor-identical newer sysroot makes every artifact
            # pass: `comm -13` yields nothing, so there is nothing to import.
            # That is indistinguishable from a real clean result unless the
            # comparison says what it compared, so it says it -- and refuses
            # rather than reporting a pass over an empty candidate set.
            if [[ "$ADDED" -eq 0 ]]; then
                err "newer sysroot $MIGO_OHOS_NEWER_SYSROOT adds no symbol over the floor"
                err "($NEWER_TOTAL exported); a comparison with nothing to find is not evidence"
                exit 1
            fi
            info "newer sysroot: $ADDED symbol(s) added after the floor, none may be imported"

            if [[ "$VCOUNT" -gt 0 ]]; then
                err "$(basename "$artifact") imports $VCOUNT symbol(s) added after the floor:"
                echo "$VIOLATIONS" | sed 's/^/    /' >&2
                TOTAL_VIOLATIONS=$((TOTAL_VIOLATIONS + VCOUNT))
            fi
        else
            info "MIGO_OHOS_NEWER_SYSROOT set but $NEWER_LIB is absent; skipping the"
            info "added-after-floor comparison (the floor-resolution count above still holds)"
        fi
    else
        info "set MIGO_OHOS_NEWER_SYSROOT to a newer SDK's native/sysroot to also"
        info "detect symbols added after the floor"
    fi

    rm -f "$ARTIFACT_UNDEF"
    ARTIFACT_UNDEF=""
done

if [[ "$TOTAL_VIOLATIONS" -gt 0 ]]; then
    err "$TOTAL_VIOLATIONS symbol(s) require an OpenHarmony newer than the declared floor"
    exit 1
fi
ok "no imported symbol postdates the floor sysroot"
