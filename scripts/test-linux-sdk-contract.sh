#!/usr/bin/env bash
# The Linux SDK's contract gate (docs/multiplatform-architecture.md 7.2).
#
# Every check fails the build outright. There is no warn-and-continue path: an
# artifact that silently misses the loader floor is worse than no artifact,
# because the consumer discovers it at load time on a machine we cannot see.
#
# Usage:
#   scripts/test-linux-sdk-contract.sh            # skips shared-object checks
#                                                 # when no libmigo.so is staged
#   scripts/test-linux-sdk-contract.sh --strict   # any skip is a failure
#
# --strict is the mode the C ABI v1 freeze gate runs in.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PREFIX="${MIGO_PREFIX:-$REPO_ROOT/dist/migo-linux-x86_64}"
AUDIT="$SCRIPT_DIR/abi-floor-audit.py"
STRICT=0
[[ "${1:-}" == "--strict" ]] && STRICT=1

FAILURES=0
SKIPS=0
pass() { echo -e "\033[0;32mPASS\033[0m  $*"; }
fail() { echo -e "\033[0;31mFAIL\033[0m  $*"; FAILURES=$((FAILURES + 1)); }
skip() { echo -e "\033[0;33mSKIP\033[0m  $*"; SKIPS=$((SKIPS + 1)); }

[[ -d "$PREFIX" ]] || {
    echo "no staged package at $PREFIX; run scripts/build-linux-sdk.sh" >&2
    exit 1
}

MANIFEST="$PREFIX/share/migo/linux-x86_64-manifest.json"
SHARED_LIB="$PREFIX/lib/libmigo.so"
FLOOR_BINARY="$REPO_ROOT/engine/target/x86_64-unknown-linux-gnu/release/migo-c-host"

# --- 1. Loader floor -------------------------------------------------------
# Measured on the sysroot-built reference consumer: symbol versions bind at link
# time, so a static archive has none of its own to check.
if [[ -x "$FLOOR_BINARY" ]]; then
    if python3 "$AUDIT" floor "$FLOOR_BINARY"; then
        pass "loader floor (sysroot-built reference consumer)"
    else
        fail "loader floor: $FLOOR_BINARY requires symbols above GLIBC_2.31 / GLIBCXX_3.4.28"
    fi
else
    fail "no sysroot-built reference consumer at $FLOOR_BINARY; run scripts/build-linux-sdk.sh"
fi

if [[ -f "$SHARED_LIB" ]]; then
    if python3 "$AUDIT" floor "$SHARED_LIB"; then
        pass "loader floor (libmigo.so)"
    else
        fail "loader floor: libmigo.so requires symbols above the floor"
    fi
fi

# --- 2. Export surface -----------------------------------------------------
# The shared library must expose the documented ABI and nothing else. Leaking a
# Rust, V8, Skia or ICU symbol would let a host bind to it, turning an internal
# change into an ABI break.
if [[ -f "$SHARED_LIB" ]]; then
    DECLARED="$(grep -ohE '\bmigo_[a-z0-9_]+[[:space:]]*\(' "$REPO_ROOT"/include/migo/*.h \
        | tr -d '( \t' | sort -u)"
    EXPORTED="$(python3 "$AUDIT" exports "$SHARED_LIB" | sort -u)"
    if [[ "$DECLARED" == "$EXPORTED" ]]; then
        pass "export surface is exactly the declared migo_* set"
    else
        fail "export surface differs from the headers:"
        diff <(echo "$DECLARED") <(echo "$EXPORTED") || true
    fi
else
    skip "export surface (no libmigo.so staged yet)"
fi

# --- 3. soname and version chain -------------------------------------------
if [[ -f "$SHARED_LIB" ]]; then
    SONAME="$(objdump -p "$SHARED_LIB" | awk '/SONAME/ {print $2}')"
    if [[ "$SONAME" == "libmigo.so.1" ]]; then
        pass "soname is libmigo.so.1"
    else
        fail "soname is '$SONAME', expected libmigo.so.1"
    fi
    if [[ -L "$PREFIX/lib/libmigo.so" && -L "$PREFIX/lib/libmigo.so.1" ]]; then
        pass "version symlink chain intact"
    else
        fail "version symlink chain incomplete under $PREFIX/lib"
    fi
else
    skip "soname and version chain (no libmigo.so staged yet)"
fi

# --- 4. Declared dependencies match reality --------------------------------
# The manifest is a claim this gate verifies, not documentation that drifts.
if [[ -f "$MANIFEST" ]]; then
    AUDIT_TARGET="$SHARED_LIB"
    [[ -f "$AUDIT_TARGET" ]] || AUDIT_TARGET="$FLOOR_BINARY"
    if [[ -e "$AUDIT_TARGET" ]]; then
        DECLARED_DEPS="$(python3 -c '
import json, sys
print("\n".join(json.load(open(sys.argv[1]))["dynamic_dependencies"]))' "$MANIFEST" | sort)"
        ACTUAL_DEPS="$(python3 "$AUDIT" needed "$AUDIT_TARGET" | sort)"
        if [[ "$DECLARED_DEPS" == "$ACTUAL_DEPS" ]]; then
            pass "manifest dependency list matches DT_NEEDED of $(basename "$AUDIT_TARGET")"
        else
            fail "manifest dependency list does not match DT_NEEDED:"
            diff <(echo "$DECLARED_DEPS") <(echo "$ACTUAL_DEPS") || true
        fi
    else
        fail "nothing to cross-check the manifest against"
    fi
else
    fail "no artifact manifest at $MANIFEST"
fi

# --- 5. Staged headers compile standalone ----------------------------------
if MIGO_INCLUDE_DIR="$PREFIX/include" bash "$SCRIPT_DIR/test-c-abi-surface-candidate.sh" \
        >/dev/null 2>&1; then
    pass "staged headers compile standalone under C11 and C++17"
else
    fail "staged headers do not compile standalone"
    MIGO_INCLUDE_DIR="$PREFIX/include" bash "$SCRIPT_DIR/test-c-abi-surface-candidate.sh" || true
fi

echo
if (( STRICT && SKIPS )); then
    echo "FAIL: --strict was requested but $SKIPS check(s) were skipped"
    exit 1
fi
if (( FAILURES )); then
    echo "FAIL: $FAILURES contract violation(s)"
    exit 1
fi
echo "OK: Linux SDK contract satisfied ($SKIPS skipped)"
