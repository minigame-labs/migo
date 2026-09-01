#!/usr/bin/env bash
# scripts/test-android-frame-rate-contract.sh
#
# Assert that the Android surface asks the display for a game's frame rate the
# way the NDK defines it, and that the value it passes stays covered by a test
# that actually runs.
#
# WHY THIS EXISTS (observed, not hypothetical):
# `request_frame_rate` passed ANATIVEWINDOW_FRAME_RATE_COMPATIBILITY_FIXED_SOURCE
# from the day it was written, and a comment above the constant argued the case
# for it: "content presenting at a fixed rate, which a game asking for N fps is".
# The NDK says the opposite in as many words -- FIXED_SOURCE is for content with
# an inherently fixed rate that forces pull-down when the system picks another,
# i.e. video, and DEFAULT "should be used when displaying game content, UIs, and
# anything that isn't video". The wrong value was not a typo; it was a reasoned
# position written into a comment, which is why deleting the comment was part of
# the fix and why a test alone would not have held: the next person to read that
# comment would have changed the value back.
#
# WHY IT CHECKS WHAT IT CHECKS:
# `platform/src/android/**` compiles only on Android, so nothing inside it is
# ever *executed* by a host test -- this repo has already shipped an ILP32
# assertion lane that never compiled and a conformance suite that reported "0
# assertions" instead of failing. The value therefore lives in a pure module
# (`android/frame_rate.rs`) that host tests do run, and the two checks below
# close the two ways that arrangement can rot:
#
#   1. `surface.rs` stops calling the tested core and inlines a number again --
#      the host test would still pass while the device does something else.
#   2. `lib.rs` narrows the module's cfg to Android only -- the test stops being
#      compiled at all and goes green by never running.
#
# The third check is unrelated to correctness of the value and guards the load
# floor: `ANativeWindow_setFrameRate` is API 30. Declaring it in the statically
# linked `#[link(name = "android")]` block would make the whole `.so` fail to
# load on every device below Android 11 -- trading the engine for a pacing hint.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SURFACE="$REPO_ROOT/engine/crates/platform/src/android/surface.rs"
POLICY="$REPO_ROOT/engine/crates/platform/src/android/frame_rate.rs"
LIBRS="$REPO_ROOT/engine/crates/platform/src/lib.rs"
TAG='[frame-rate]'

pass() { echo -e "\033[0;32m$TAG PASS $*\033[0m"; }
fail() { echo -e "\033[0;31m$TAG FAIL $*\033[0m" >&2; failures=$((failures + 1)); }
info() { echo -e "\033[0;36m$TAG $*\033[0m"; }
failures=0

for f in "$SURFACE" "$POLICY" "$LIBRS"; do
    [[ -f "$f" ]] || { echo "$TAG missing source: $f" >&2; exit 1; }
done

# ---------------------------------------------------------------------------
info "the surface passes the tested policy, not a literal"
# ---------------------------------------------------------------------------
call_line="$(grep -n 'set_frame_rate(window' "$SURFACE" || true)"
if [[ -z "$call_line" ]]; then
    fail "no call to set_frame_rate(window, ...) found in $SURFACE"
elif grep -q 'set_frame_rate(window, fps as f32, game_compatibility().as_abi())' "$SURFACE"; then
    pass "request_frame_rate passes game_compatibility().as_abi()"
else
    fail "the setFrameRate call does not pass game_compatibility().as_abi():"
    fail "  $call_line"
fi

if grep -qE 'FIXED_SOURCE' "$SURFACE"; then
    fail "$SURFACE still names FIXED_SOURCE; the value belongs in frame_rate.rs"
else
    pass "no FIXED_SOURCE constant or comment left in the surface"
fi

# ---------------------------------------------------------------------------
info "the policy a game gets is DEFAULT, and both ABI values are written down"
# ---------------------------------------------------------------------------
if grep -qE 'fn game_compatibility\(\) -> FrameRateCompatibility \{' "$POLICY" \
   && grep -A1 'fn game_compatibility() -> FrameRateCompatibility {' "$POLICY" \
      | grep -q 'FrameRateCompatibility::Default'; then
    pass "game_compatibility() returns Default"
else
    fail "game_compatibility() does not return FrameRateCompatibility::Default"
fi

if grep -qE '^\s*Default = 0,' "$POLICY" && grep -qE '^\s*FixedSource = 1,' "$POLICY"; then
    pass "the ABI values match libandroid (DEFAULT=0, FIXED_SOURCE=1)"
else
    fail "the FrameRateCompatibility discriminants no longer match the NDK ABI"
fi

# ---------------------------------------------------------------------------
info "the policy stays reachable from host tests"
# ---------------------------------------------------------------------------
# Without `test` in the cfg the module is Android-only: the unit test stops
# being compiled and the lane goes green by never running.
if grep -B2 'mod android_frame_rate;' "$LIBRS" \
   | grep -q '#\[cfg(any(target_os = "android", test))\]'; then
    pass "android_frame_rate is compiled for host tests as well as Android"
else
    fail "android_frame_rate is not gated on \`any(target_os = \"android\", test)\`;"
    fail "  its unit test would stop running instead of failing"
fi

# ---------------------------------------------------------------------------
info "the API 30 entry point stays dynamically resolved"
# ---------------------------------------------------------------------------
# Everything between `#[link(name = "android")]` and the closing brace of that
# extern block is linked at load time and must exist in the API 26 stub.
link_block="$(awk '/#\[link\(name = "android"\)\]/{inblock=1} inblock{print} inblock&&/^\}/{exit}' "$SURFACE")"
if [[ -z "$link_block" ]]; then
    fail "could not locate the #[link(name = \"android\")] extern block in $SURFACE"
elif grep -q 'ANativeWindow_setFrameRate' <<<"$link_block"; then
    fail "ANativeWindow_setFrameRate is declared in the statically linked extern block;"
    fail "  it is API 30 and this library must load on API 26"
else
    pass "ANativeWindow_setFrameRate is not statically linked"
fi

if grep -q 'ANativeWindow_setFrameRate\\0' "$SURFACE"; then
    pass "it is resolved through a runtime symbol lookup"
else
    fail "no runtime lookup of ANativeWindow_setFrameRate found"
fi

echo
if (( failures > 0 )); then
    echo -e "\033[0;31m$TAG $failures check(s) failed\033[0m" >&2
    exit 1
fi
echo -e "\033[0;32m$TAG all checks passed\033[0m"
exit 0
