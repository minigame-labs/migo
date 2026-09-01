#!/usr/bin/env bash
# scripts/test-android-sdk-levels-contract.sh
#
# Assert the three Android SDK levels the AAR declares, and that they stay three
# independent facts.
#
# WHY THIS EXISTS (observed, not hypothetical):
# Nothing in this repository checked them. `scripts/test-android-sdk-contract.sh`
# has the closest name but is about the published C ABI package -- symbols,
# snapshot, layout -- and never reads build.gradle. So `compileSdk`/`targetSdk`
# sat at 34 while Google Play began requiring API 36 for new apps and updates on
# 2026-08-31, and the only thing that would have caught it was somebody
# remembering.
#
# WHY IT CHECKS WHAT IT CHECKS:
# `minSdk` is in here for the opposite reason to the other two. Raising target is
# a publishing requirement; raising min drops devices, and the cheapest way to
# make a target bump "work" is to drag min up with it until the errors stop.
# This engine's floor of 26 is a product decision backed by device reach, not a
# build convenience, so the gate fails on a min that moved *in either direction*
# -- a silent drop to 24 would be just as much a change nobody approved.
#
# The gate reads the declarations rather than a built artifact so it can run in
# the source-only lane, where the Android SDK platform may not be installed.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
GRADLE="$REPO_ROOT/platforms/android/library/build.gradle"
TAG='[sdk-levels]'

# The contract. Changing a number here is a deliberate act that shows up in
# review as a change to the contract, not as a drifting build file.
WANT_COMPILE_SDK=36
WANT_TARGET_SDK=36
WANT_MIN_SDK=26

pass() { echo -e "\033[0;32m$TAG PASS $*\033[0m"; }
fail() { echo -e "\033[0;31m$TAG FAIL $*\033[0m" >&2; failures=$((failures + 1)); }
info() { echo -e "\033[0;36m$TAG $*\033[0m"; }
failures=0

[[ -f "$GRADLE" ]] || { echo "$TAG missing $GRADLE" >&2; exit 1; }

# Reads `<key> <number>` or `<key> = <number>`, ignoring commented-out lines.
read_level() { # <key>
    grep -oE "^[[:space:]]*$1[[:space:]]*=?[[:space:]]*[0-9]+" "$GRADLE" \
        | grep -oE '[0-9]+$' | head -1
}

check_level() { # <key> <want> <why>
    local key="$1" want="$2" why="$3" got
    got="$(read_level "$key")"
    if [[ -z "$got" ]]; then
        fail "$key is not declared in library/build.gradle"
    elif [[ "$got" == "$want" ]]; then
        pass "$key = $want"
    else
        fail "$key = $got, expected $want -- $why"
    fi
}

info "the AAR declares the SDK levels this project has decided on"
check_level compileSdk "$WANT_COMPILE_SDK" \
    "compiling against an older SDK hides the behaviour changes target $WANT_TARGET_SDK opts into"
check_level targetSdk "$WANT_TARGET_SDK" \
    "Google Play has required API $WANT_TARGET_SDK for new apps and updates since 2026-08-31"
check_level minSdk "$WANT_MIN_SDK" \
    "the API $WANT_MIN_SDK floor is a device-reach decision; moving it in either direction needs its own case"

info "target and min stay independent"
if [[ "$WANT_MIN_SDK" -ge "$WANT_TARGET_SDK" ]]; then
    fail "the contract itself now demands min >= target, which cannot be right"
else
    pass "the contract keeps min ($WANT_MIN_SDK) below target ($WANT_TARGET_SDK)"
fi

echo
if (( failures > 0 )); then
    echo -e "\033[0;31m$TAG $failures check(s) failed\033[0m" >&2
    exit 1
fi
echo -e "\033[0;32m$TAG all checks passed\033[0m"
exit 0
