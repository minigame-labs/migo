#!/usr/bin/env bash
# ============================================================
# Device counterpart to scripts/verify-state-shadow.sh.
# Location: scripts/verify-state-shadow-device.sh
#
# Same fixture, same three assertions (frame count, dominant colour, distinct
# colour count), read off a real screencap instead of MIGO_PLAYER_PNG. See
# that script for what each of the three shadow reshapes (texture-unit
# bindings, capability toggles, vertex-attribute state) is defending.
#
# Usage:
#   scripts/verify-state-shadow-device.sh [SECS] [--device SERIAL]
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

PASS="51,204,102,255"     # every shadow deduped only what was redundant
LOST_BINDING="0,0,0,255"  # unit 3 unbound, or attribute 2 left disabled
NO_WRITE="217,26,38,255"  # the draw wrote nothing; the blend disable was deduped
FROZEN="26,77,230,255"    # frame one still on the surface

echo
echo "STATE SHADOW ON DEVICE  ${SECS}s, expecting rgba($PASS)"
if ! run_fixture_on_device state-shadow-probe "$SECS" "$DEVICE"; then
    echo "could not run the probe on device" >&2
    exit 1
fi
printf '  frames=%-5s pixel=%s (%s colour(s))\n' "$RF_FRAMES" "$RF_COLOUR" "$RF_DISTINCT"
echo "  log: $RF_LOG"
echo "  png: $RF_PNG"

if [[ "$RF_FRAMES" -lt 2 ]]; then
    echo "painted $RF_FRAMES frame(s); need at least two to draw once. See $RF_LOG" >&2
    exit 1
fi

case "$RF_COLOUR" in
    "$PASS") ;;
    "$LOST_BINDING")
        echo "a shadow over-deduped: unit 3 unbound or attribute 2 mis-indexed. See TextureUnitShadow / VertexArrayShadow in canvas/manager/types.rs." >&2
        exit 1
        ;;
    "$NO_WRITE")
        echo "the draw wrote nothing: glDisable(BLEND) was deduped away. See CapabilityShadow in canvas/manager/types.rs." >&2
        exit 1
        ;;
    "$FROZEN")
        echo "frame one is still on the surface; presentation stopped. See $RF_LOG" >&2
        exit 1
        ;;
    *)
        echo "unexpected dominant pixel $RF_COLOUR; expected $PASS / $LOST_BINDING / $NO_WRITE / $FROZEN. See $RF_PNG" >&2
        exit 1
        ;;
esac

if [[ "$RF_DISTINCT" -ne 1 ]]; then
    echo "the surface carries $RF_DISTINCT distinct colours; the fixture paints one flat colour. See $RF_PNG" >&2
    exit 1
fi

echo "OK: the GL state shadows dedup only what is redundant on device ($RF_FRAMES frames, flat $RF_COLOUR)"
