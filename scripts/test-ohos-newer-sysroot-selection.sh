#!/usr/bin/env bash
# =============================================================================
# Contract: which OpenHarmony SDK the API floor gate compares against.
#
# The floor gate answers "does this archive import anything the floor lacks".
# Only a *newer* sysroot can answer "did any of these symbols arrive after the
# floor", and picking the wrong one does not produce an error -- it produces a
# confident pass computed against the wrong set. That is why this is gated
# rather than left to review: the selection has exactly one observable output
# and every wrong rule still prints a plausible path.
#
# The layouts below are chosen so that a name-ordering rule and an apiVersion
# rule DISAGREE. A fixture where both rules give the same answer would pass
# against either implementation and prove nothing.
#
# Fails closed: every case asserts an exact expected value, and the run reports
# how many cases it executed, so an empty run cannot be mistaken for a clean one.
# =============================================================================
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SELECT="$SCRIPT_DIR/lib/select-ohos-newer-sysroot.py"

PASS_COUNT=0
FAIL_COUNT=0

pass() { echo -e "\033[0;32mPASS\033[0m  $*"; PASS_COUNT=$((PASS_COUNT + 1)); }
fail() { echo -e "\033[0;31mFAIL\033[0m  $*" >&2; FAIL_COUNT=$((FAIL_COUNT + 1)); }

if [[ ! -f "$SELECT" ]]; then
    echo "error: selector not found at $SELECT" >&2
    exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Builds a fake SDK: a declared apiVersion plus the native/sysroot directory the
# selector requires to exist. `omit-sysroot` builds one without it.
make_sdk() {
    local home="$1" api="$2" mode="${3:-}"
    mkdir -p "$home/native"
    if [[ "$mode" != "omit-sysroot" ]]; then
        mkdir -p "$home/native/sysroot"
    fi
    if [[ "$mode" != "omit-api" ]]; then
        printf '{"apiVersion": "%s", "version": "fixture"}\n' "$api" > "$home/native/oh-uni-package.json"
    else
        printf '{"version": "fixture"}\n' > "$home/native/oh-uni-package.json"
    fi
}

# Asserts the selector's stdout for a given floor home.
expect() {
    local name="$1" floor="$2" want="$3"
    local got
    got="$(python3 "$SELECT" "$floor" 2>/dev/null)"
    if [[ "$got" == "$want" ]]; then
        pass "$name"
    else
        fail "$name -- expected '${want:-<nothing>}', got '${got:-<nothing>}'"
    fi
}

# --- Case 1: a newer SDK exists and must be chosen -------------------------
C1="$WORK/c1"; mkdir -p "$C1"
make_sdk "$C1/ohos-sdk" 18
make_sdk "$C1/ohos-sdk-6.1" 23
expect "picks the SDK declaring a higher API" "$C1/ohos-sdk" "$C1/ohos-sdk-6.1/native/sysroot"

# --- Case 2: the floor IS the newest -- nothing may be returned ------------
# A highest-sorted-directory rule returns ohos-sdk-5.1 here and its extra
# symbols would be reported as post-floor, which is evidence pointing backwards.
C2="$WORK/c2"; mkdir -p "$C2"
make_sdk "$C2/ohos-sdk" 23
make_sdk "$C2/ohos-sdk-5.1" 18
expect "refuses an older sibling when the floor is newest" "$C2/ohos-sdk" ""

# --- Case 3: name order and API order disagree ------------------------------
# ohos-sdk-9.9 sorts last but declares a LOWER API than the floor. Only an
# apiVersion rule rejects it; a name rule takes it. This is the case that makes
# the fixture load-bearing.
C3="$WORK/c3"; mkdir -p "$C3"
make_sdk "$C3/ohos-sdk" 18
make_sdk "$C3/ohos-sdk-6.1" 23
make_sdk "$C3/ohos-sdk-9.9" 15
expect "ignores a higher-sorting directory declaring a lower API" \
    "$C3/ohos-sdk" "$C3/ohos-sdk-6.1/native/sysroot"

# --- Case 4: highest API wins among several newer SDKs ---------------------
C4="$WORK/c4"; mkdir -p "$C4"
make_sdk "$C4/ohos-sdk" 18
make_sdk "$C4/ohos-sdk-6.1" 23
make_sdk "$C4/ohos-sdk-alpha" 30
expect "picks the highest declared API among several candidates" \
    "$C4/ohos-sdk" "$C4/ohos-sdk-alpha/native/sysroot"

# --- Case 5: a candidate without native/sysroot is not a sysroot -----------
# ~/ohos-sdk-dl (a download cache) matches the ohos-sdk* glob on a real machine.
C5="$WORK/c5"; mkdir -p "$C5"
make_sdk "$C5/ohos-sdk" 18
make_sdk "$C5/ohos-sdk-dl" 23 omit-sysroot
expect "skips a candidate with no native/sysroot" "$C5/ohos-sdk" ""

# --- Case 6: a candidate that declares no API cannot be compared -----------
C6="$WORK/c6"; mkdir -p "$C6"
make_sdk "$C6/ohos-sdk" 18
make_sdk "$C6/ohos-sdk-unknown" 0 omit-api
expect "skips a candidate declaring no apiVersion" "$C6/ohos-sdk" ""

# --- Case 7: an undeclared floor makes "newer" meaningless -----------------
C7="$WORK/c7"; mkdir -p "$C7"
make_sdk "$C7/ohos-sdk" 0 omit-api
make_sdk "$C7/ohos-sdk-6.1" 23
expect "returns nothing when the floor declares no apiVersion" "$C7/ohos-sdk" ""

# --- Case 8: no siblings at all --------------------------------------------
C8="$WORK/c8"; mkdir -p "$C8"
make_sdk "$C8/ohos-sdk" 18
expect "returns nothing when no second SDK is installed" "$C8/ohos-sdk" ""

echo
if [[ "$FAIL_COUNT" -gt 0 ]]; then
    echo -e "\033[0;31mFAIL: OpenHarmony newer-sysroot selection ($PASS_COUNT passed, $FAIL_COUNT failed)\033[0m" >&2
    exit 1
fi
if [[ "$PASS_COUNT" -lt 8 ]]; then
    echo -e "\033[0;31mFAIL: only $PASS_COUNT case(s) ran; the fixture did not execute\033[0m" >&2
    exit 1
fi
echo -e "\033[0;32mOK: OpenHarmony newer-sysroot selection ($PASS_COUNT cases)\033[0m"
