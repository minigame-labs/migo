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
#include <migo/platform/openharmony.h>
#include <stdio.h>

/*
 * This consumer references the attach entry point on purpose, and the reason is
 * a bug this check previously missed rather than thoroughness.
 *
 * A consumer that only calls migo_query_capabilities lets --gc-sections discard
 * the entire surface backend, so the link succeeds whether or not the package
 * names the libraries that backend needs. The published CMake package was in
 * fact missing native_window: every gate passed, and the first third party to
 * attach a window would have hit two undefined OH_NativeWindow_* symbols.
 *
 * Referencing attach keeps the backend live, which makes a missing link library
 * a link failure here instead of a failure in someone else's build. Nothing is
 * executed -- this host cannot run an OpenHarmony binary -- so passing a null
 * session is safe and never reaches the engine.
 */
int main(void) {
    MigoCapabilities caps = {0};
    caps.struct_size = (uint32_t)sizeof(caps);
    caps.abi_version = MIGO_ABI_VERSION_1;
    if (migo_query_capabilities(&caps) != MIGO_OK) {
        return 1;
    }

    MigoOpenHarmonyNativeWindowDescriptor win = {0};
    win.struct_size = (uint32_t)sizeof(win);
    win.abi_version = MIGO_ABI_VERSION_1;
    win.platform_kind = MIGO_PLATFORM_OPENHARMONY_NATIVE_WINDOW;

    MigoSurfaceDescriptor surface = {0};
    surface.struct_size = (uint32_t)sizeof(surface);
    surface.abi_version = MIGO_ABI_VERSION_1;
    surface.platform_kind = MIGO_PLATFORM_OPENHARMONY_NATIVE_WINDOW;
    surface.platform_descriptor_size = (uint32_t)sizeof(win);
    surface.platform_descriptor = &win;

    printf("%u %u %llu %p %u\n", caps.abi_version_min, caps.abi_version_max,
           (unsigned long long)caps.platform_kinds,
           (void *)&migo_session_attach_surface, surface.platform_kind);
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

# ---- 6. the capability claim matches the bytes ------------------------------
# This is the check the Windows incident needed: that SDK declared a surface
# descriptor, pinned its layout on both pointer widths, agreed with itself
# everywhere, and had no implementation behind any of it.
#
# The claim is checked against the archive rather than against a header, a macro
# or a list in the build script, because only the archive is downstream of
# whether the code was actually compiled in. The evidence is that a surface
# backend cannot exist without calling the platform to reference the window it
# was handed: an archive with a backend imports
# OH_NativeWindow_NativeObjectReference and does not define it, and an archive
# without one imports nothing of the sort. A header-only descriptor -- exactly
# the Windows failure -- imports nothing and is caught here.
#
# This host cannot execute an OpenHarmony binary to ask the library directly
# (the SDK sysroot ships a link stub, not a loadable musl loader), so this is
# the strongest available evidence rather than a convenience.
#
# What each direction is actually proven against, so nobody has to guess:
#   understating (backend present, manifest empty) -- fires here, and did so for
#     real on a stale aarch64 archive built before the backend existed.
#   overstating (manifest claims, backend absent) -- the consumer link above
#     fires first: deleting the object that holds the window import from a copy
#     of the package makes the link fail, because the consumer references
#     attach. This branch is the backstop for shapes that still link.
LIB="$PREFIX/lib/libmigo_capi.a"
IMPL_SYMBOL_PRESENT="$(nm --defined-only "$LIB" 2>/dev/null \
    | grep -c 'migo_query_capabilities' || true)"
if [[ "$IMPL_SYMBOL_PRESENT" -eq 0 ]]; then
    fail "migo_query_capabilities is absent; the capability claim cannot be checked"
    exit 1
fi

WINDOW_API="OH_NativeWindow_NativeObjectReference"
BACKEND_IMPORTS="$(nm --undefined-only "$LIB" 2>/dev/null | grep -c "$WINDOW_API" || true)"
# A definition inside the archive would make the import test vacuous: the symbol
# would be satisfied internally and prove nothing about a platform backend.
BACKEND_DEFINES="$(nm --defined-only "$LIB" 2>/dev/null | grep -c "$WINDOW_API" || true)"
if [[ "$BACKEND_DEFINES" -ne 0 ]]; then
    fail "$WINDOW_API is defined inside the archive; this test cannot distinguish"
    fail "a real platform backend from a stub and must be rewritten before it is trusted"
    exit 1
fi

DECLARED_KINDS="$(python3 -c '
import json,sys
d=json.load(open(sys.argv[1]))
print(len(d.get("capabilities",{}).get("attachable_platform_kinds",[])))' "$MANIFEST")"

if [[ "$BACKEND_IMPORTS" -gt 0 && "$DECLARED_KINDS" -eq 0 ]]; then
    fail "the archive has a surface backend (imports $WINDOW_API) but the manifest"
    fail "claims no attachable kind -- the package understates what it can do"
    exit 1
fi
if [[ "$BACKEND_IMPORTS" -eq 0 && "$DECLARED_KINDS" -gt 0 ]]; then
    fail "the manifest claims $DECLARED_KINDS attachable kind(s), but the archive imports"
    fail "no OpenHarmony window API, so no surface backend is compiled into it."
    fail "This is the failure mode a published SDK already shipped once: every"
    fail "declaration agreeing with every other declaration, and nothing behind them."
    exit 1
fi
if [[ "$BACKEND_IMPORTS" -gt 0 ]]; then
    pass "manifest claims $DECLARED_KINDS attachable kind(s), and the archive has the backend to match"
else
    pass "manifest claims no attachable kind, and the archive has no backend (consistent)"
fi

pass "OpenHarmony SDK package contract holds for $ARCH"
