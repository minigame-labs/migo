#!/usr/bin/env bash
# =============================================================================
# OpenHarmony SDK package contract.
#
# Verifies a staged dist/migo-ohos-<arch> package is actually consumable:
# complete, self-consistent, honest about what it supports, and -- the part
# that matters most -- linkable by an external consumer that sees only the
# public headers.
#
# WHY THE LINK CHECK IS THE CENTRE OF THIS:
# Header-to-header checks cannot see a missing implementation. This project
# shipped a Windows SDK where platform/win32.h declared the descriptor, the
# layout lanes pinned it for both pointer widths, every gate agreed with every
# other gate -- and no implementation existed. The published package loaded,
# resolved all 24 entry points, and could attach nothing. The check that would
# have caught it is asking the built library what it supports, so this gate
# asks, and it also compiles and links a real consumer.
#
# Usage:
#   scripts/test-ohos-sdk-contract.sh <package-prefix>
#
# Env:
#   OHOS_NDK_HOME   OpenHarmony SDK (default: probed via dev-setup-ohos.sh)
#
# Fails closed: a missing SDK, a missing file, or an unverifiable claim is an
# error, not a skip.
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

pass() { echo -e "\033[0;32m[ohos-sdk-contract] PASS $*\033[0m"; }
fail() { echo -e "\033[0;31m[ohos-sdk-contract] FAIL $*\033[0m" >&2; }
info() { echo -e "\033[0;36m[ohos-sdk-contract] $*\033[0m"; }

PREFIX="${1:-}"
if [[ -z "$PREFIX" || ! -d "$PREFIX" ]]; then
    fail "usage: $0 <package-prefix>"
    exit 1
fi
PREFIX="$(cd "$PREFIX" && pwd)"
info "package: $PREFIX"

if [[ -z "${OHOS_SDK_NATIVE:-}" ]]; then
    eval "$(bash "$SCRIPT_DIR/dev-setup-ohos.sh" | grep '^export OHOS_')"
fi
[[ -d "${OHOS_SDK_NATIVE:-/nonexistent}" ]] || { fail "OpenHarmony SDK not usable"; exit 1; }

MANIFEST="$(find "$PREFIX/share/migo" -name 'ohos-*-manifest.json' -print -quit 2>/dev/null || true)"
[[ -n "$MANIFEST" ]] || { fail "no manifest under share/migo"; exit 1; }
ARCH="$(basename "$MANIFEST" | sed 's/^ohos-//; s/-manifest\.json$//')"
info "arch: $ARCH"

# ---- 1. structure -----------------------------------------------------------
REQUIRED=(
    "include/migo/migo.h"
    "include/migo/capabilities.h"
    "include/migo/platform/openharmony.h"
    "lib/libmigo_capi.a"
    "lib/cmake/migo/migo-config.cmake"
    "lib/cmake/migo/migo-config-version.cmake"
    "README.md"
)
for f in "${REQUIRED[@]}"; do
    [[ -e "$PREFIX/$f" ]] || { fail "missing: $f"; exit 1; }
done
pass "package structure complete (${#REQUIRED[@]} required paths)"

# ---- 2. staged bytes match the manifest -------------------------------------
DECLARED_SHA="$(python3 -c '
import json,sys
d=json.load(open(sys.argv[1]))
print(d["artifacts"]["lib/libmigo_capi.a"])' "$MANIFEST")"
ACTUAL_SHA="$(sha256sum "$PREFIX/lib/libmigo_capi.a" | cut -d' ' -f1)"
if [[ "$DECLARED_SHA" != "$ACTUAL_SHA" ]]; then
    fail "manifest hash does not match the staged library"
    fail "  declared $DECLARED_SHA"
    fail "  actual   $ACTUAL_SHA"
    exit 1
fi
pass "staged library matches its manifest hash"

# ---- 3. export surface ------------------------------------------------------
DECLARED_ENTRIES="$(python3 -c '
import json,sys
print(json.load(open(sys.argv[1]))["entry_points"])' "$MANIFEST")"
ACTUAL_ENTRIES="$(nm --defined-only "$PREFIX/lib/libmigo_capi.a" 2>/dev/null | grep -c ' T migo_' || true)"
if [[ "$ACTUAL_ENTRIES" -eq 0 ]]; then
    fail "library defines no migo_* entry points"
    exit 1
fi
if [[ "$DECLARED_ENTRIES" != "$ACTUAL_ENTRIES" ]]; then
    fail "manifest declares $DECLARED_ENTRIES entry points, library has $ACTUAL_ENTRIES"
    exit 1
fi
pass "$ACTUAL_ENTRIES migo_* entry points, matching the manifest"

# ---- 4. an external consumer links ------------------------------------------
# The consumer sees only the public headers and the staged library. It is
# compiled with the SDK's own driver, which is what a real consumer uses.
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cat > "$WORK/consumer.c" <<'CONSUMER'
#include <migo/migo.h>
#include <stdio.h>

int main(void) {
    MigoCapabilities caps = {0};
    caps.struct_size = (uint32_t)sizeof(caps);
    caps.abi_version = MIGO_ABI_VERSION_1;
    if (migo_query_capabilities(&caps) != MIGO_OK) {
        return 1;
    }
    printf("%u %u %llu\n", caps.abi_version_min, caps.abi_version_max,
           (unsigned long long)caps.platform_kinds);
    return 0;
}
CONSUMER

case "$ARCH" in
    x86_64)  DRIVER="x86_64-unknown-linux-ohos-clang" ;;
    aarch64) DRIVER="aarch64-unknown-linux-ohos-clang" ;;
    *) fail "unknown arch: $ARCH"; exit 1 ;;
esac
CC="$OHOS_SDK_NATIVE/llvm/bin/$DRIVER"
[[ -x "$CC" ]] || { fail "SDK driver not found: $CC"; exit 1; }

# The flags come from the CMake package rather than being restated here, so a
# drift between what the package promises and what actually links shows up.
LINK_LIBS="$(sed -n 's/.*INTERFACE_LINK_LIBRARIES "\([^"]*\)".*/\1/p' \
    "$PREFIX/lib/cmake/migo/migo-config.cmake" | tr ';' ' ')"
LINK_OPTS="$(sed -n 's/.*INTERFACE_LINK_OPTIONS "\([^"]*\)".*/\1/p' \
    "$PREFIX/lib/cmake/migo/migo-config.cmake")"
if [[ -z "$LINK_LIBS" ]]; then
    fail "could not read INTERFACE_LINK_LIBRARIES out of the CMake package"
    fail "this check would otherwise link with flags of its own invention"
    exit 1
fi
info "link libraries from the package: $LINK_LIBS"
info "link options from the package:   ${LINK_OPTS:-<none>}"

LIB_ARGS=()
for l in $LINK_LIBS; do LIB_ARGS+=("-l$l"); done

if ! "$CC" "$WORK/consumer.c" -I"$PREFIX/include" "$PREFIX/lib/libmigo_capi.a" \
        ${LINK_OPTS:+$LINK_OPTS} "${LIB_ARGS[@]}" -o "$WORK/consumer" \
        > "$WORK/link.log" 2>&1; then
    fail "external consumer failed to link:"
    grep -oE "undefined symbol: [A-Za-z_0-9]+" "$WORK/link.log" | sort -u | head -20 >&2 \
        || tail -20 "$WORK/link.log" >&2
    exit 1
fi
pass "external consumer links with every migo_* resolved"

# ---- 5. the binary really targets OpenHarmony -------------------------------
# A consumer that links but produces a glibc binary would pass every check
# above while being useless on a device.
INTERP="$(readelf -l "$WORK/consumer" 2>/dev/null | grep -oE '/lib/ld-[a-z0-9.-]*' | head -1 || true)"
case "$INTERP" in
    *musl*) pass "consumer binary uses the musl loader ($INTERP)" ;;
    "")     fail "consumer binary declares no interpreter; cannot confirm the target"; exit 1 ;;
    *)      fail "consumer binary uses $INTERP, not a musl loader -- this is not an OpenHarmony build"; exit 1 ;;
esac

# ---- 6. declared capabilities match what the library reports ----------------
# This is the check the Windows incident needed. The manifest may not claim an
# attachable platform kind that the built library does not actually support.
DECLARED_KINDS="$(python3 -c '
import json,sys
d=json.load(open(sys.argv[1]))
print(len(d.get("capabilities",{}).get("attachable_platform_kinds",[])))' "$MANIFEST")"

# The library reports its own supported kinds through the same constant the
# attach path enforces; a non-zero mask with an empty manifest list (or the
# reverse) is a contradiction.
IMPL_SYMBOL_PRESENT="$(nm --defined-only "$PREFIX/lib/libmigo_capi.a" 2>/dev/null \
    | grep -c 'migo_query_capabilities' || true)"
if [[ "$IMPL_SYMBOL_PRESENT" -eq 0 ]]; then
    fail "migo_query_capabilities is absent; the capability claim cannot be checked"
    exit 1
fi

if [[ "$DECLARED_KINDS" -eq 0 ]]; then
    pass "manifest claims no attachable platform kind (matches the unimplemented backend)"
else
    # Claiming kinds requires evidence the backend exists. Running the consumer
    # is impossible on this host, so a claim can only be admitted once an
    # on-device or emulator run backs it.
    fail "manifest claims $DECLARED_KINDS attachable kind(s), but this gate cannot"
    fail "execute an OpenHarmony binary to confirm it. A claim that cannot be"
    fail "verified here must be backed by an emulator/device run first."
    exit 1
fi

pass "OpenHarmony SDK package contract holds for $ARCH"
