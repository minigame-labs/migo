#!/usr/bin/env bash
# ============================================================
# Device counterpart to scripts/verify-uniform-shadow.sh.
# Location: scripts/verify-uniform-shadow-device.sh
#
# Same fixture, same three assertions (frame count, dominant colour, distinct
# colour count), read off a real screencap instead of MIGO_PLAYER_PNG -- Mesa
# is conforming and forgiving; the uniform-invalidation defect this fixture
# targets (glLinkProgram vs glUseProgram, GLES 3.0 §2.11.6) is exactly the kind
# of GL state assumption that Mali/Adreno enforce and Mesa does not.
#
# Usage:
#   scripts/verify-uniform-shadow-device.sh [SECS] [--device SERIAL]
# ============================================================
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib/device-fixture-runner.sh"

SECS=6
DEVICE="${ANDROID_SERIAL:-}"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --device) DEVICE="${2:?}"; shift 2 ;;
        --device=*) DEVICE="${1#*=}"; shift ;;
        *) SECS="$1"; shift ;;
    esac
done

PASS="51,204,102,255"   # the re-linked uniform reached the driver
DEDUPED="0,0,0,0"       # deduped against a driver holding a reset (zero) value
FROZEN="26,77,230,255"  # frame one is still on the surface

echo
echo "UNIFORM SHADOW ON DEVICE  ${SECS}s, expecting rgba($PASS)"
if ! run_fixture_on_device uniform-shadow-probe "$SECS" "$DEVICE"; then
    echo "could not run the probe on device" >&2
    exit 1
fi
printf '  frames=%-5s pixel=%s (%s colour(s))\n' "$RF_FRAMES" "$RF_COLOUR" "$RF_DISTINCT"
echo "  log: $RF_LOG"
echo "  png: $RF_PNG"

if [[ "$RF_FRAMES" -lt 2 ]]; then
    echo "painted $RF_FRAMES frame(s); need at least two to exercise a re-link. See $RF_LOG" >&2
    exit 1
fi

case "$RF_COLOUR" in
    "$PASS") ;;
    "$DEDUPED")
        echo "a re-linked program's uniform was deduped against a reset driver: the shadow invalidation is still keyed off glUseProgram instead of glLinkProgram. See state_tracker.rs (invalidate_program_uniforms)." >&2
        exit 1
        ;;
    "$FROZEN")
        echo "frame one is still on the surface; presentation stopped. See $RF_LOG" >&2
        exit 1
        ;;
    *)
        echo "unexpected dominant pixel $RF_COLOUR; expected $PASS / $DEDUPED / $FROZEN. See $RF_PNG" >&2
        exit 1
        ;;
esac

if [[ "$RF_DISTINCT" -ne 1 ]]; then
    echo "the surface carries $RF_DISTINCT distinct colours; the fixture paints one flat colour. See $RF_PNG" >&2
    exit 1
fi

echo "OK: re-linked uniforms reach the driver on device ($RF_FRAMES frames, flat $RF_COLOUR)"
