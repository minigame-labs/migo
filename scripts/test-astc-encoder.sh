#!/usr/bin/env bash
# The ASTC encoder is checked by a decoder that is not ours.
#
# ASTC is an intricate format: eleven bits of block mode whose layout is a
# five-row table, endpoint values packed as base-three trits interleaved with
# raw bits, weights written from the top of the block downwards in reverse bit
# order, and a second weight plane whose channel selector sits directly beneath
# them. Every one of those is a place to misread the specification.
#
# An encoder tested against a decoder written by the same author from the same
# specification proves the two agree. It does not prove either is right, and a
# misreading lands in both. So the oracle here is whatever ASTC decoder the GL
# stack ships -- Mesa's on this repository's Linux host, the GPU's on a device
# -- and the check is that pixels encoded and then decoded come back close to
# the pixels they started as.
#
# THE DRIFT THIS EXISTS TO CATCH is the silent kind that compressed formats
# specialise in. Nothing about a wrong block mode fails to build, fails to
# upload, or logs anything: the texture decodes to the wrong colours, on a
# device, in a game, and the first report is that the art looks off.
#
# Host-only: needs a GL stack with GL_KHR_texture_compression_astc_ldr. A host
# without one is a failure, not a skip -- it cannot answer the question, and
# reporting that as success is how a check stops being one.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ORACLE_SRC="tools/astc-oracle/oracle.c"
[[ -f "$ORACLE_SRC" ]] || { echo "FAIL: $ORACLE_SRC is missing." >&2; exit 1; }

CC_BIN="${CC:-cc}"
command -v "$CC_BIN" >/dev/null 2>&1 || { echo "FAIL: no C compiler ($CC_BIN)." >&2; exit 1; }
command -v cargo >/dev/null 2>&1 || { echo "FAIL: cargo is not available." >&2; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

if ! "$CC_BIN" "$ORACLE_SRC" -o "$WORK/oracle" -lEGL -lGLESv2 -lm 2>"$WORK/cc.log"; then
    echo "FAIL: could not build the ASTC oracle." >&2
    cat "$WORK/cc.log" >&2
    exit 1
fi

# `--nocapture` so the fixture count is in the log. A run that emitted nothing
# and passed is the shape this repository keeps finding.
emit="$(cd engine && MIGO_ASTC_FIXTURE_DIR="$WORK" \
    cargo test -p migo-io --test astc_fixtures -- --ignored --nocapture 2>&1)" || {
    printf '%s\n' "$emit" >&2
    echo "FAIL: the fixture emitter did not run." >&2
    exit 1
}
printf '%s\n' "$emit" | grep -E 'wrote [0-9]+ ASTC fixtures' || {
    echo "FAIL: the emitter did not report writing fixtures; it may not have run." >&2
    exit 1
}

# What the tolerance means, and why it is this small.
#
# Every fixture here is chosen to be representable: each 4x4 tile lies on one
# colour line, which is what one partition can hold. So the only error the
# encoder should contribute is endpoint quantisation, and the endpoints are
# stored in the range 0..47 -- about 5.4 of 255 per step, so about 3 after
# rounding. All four fixtures land at 3 or 0.
#
# Eight leaves room for a rounding change and none for a structural one. A wrong
# block mode, a mispacked trit or a weight plane read at the wrong offset does
# not produce a near miss: the first version of this encoder compared endpoint
# ordering on stored levels rather than decoded values, and that one mistake
# read as 51.
TOLERANCE="${MIGO_ASTC_TOLERANCE:-8}"

shopt -s nullglob
fixtures=("$WORK"/*.astc)
(( ${#fixtures[@]} > 0 )) || { echo "FAIL: no fixtures were written." >&2; exit 1; }

failed=0
for blocks in "${fixtures[@]}"; do
    name="$(basename "$blocks" .astc)"
    read -r width height < "$WORK/$name.size"
    echo "- $name"
    if ! "$WORK/oracle" "$blocks" "$WORK/$name.rgba" "$width" "$height" "$TOLERANCE"; then
        status=$?
        if (( status == 4 )); then
            echo "FAIL: this host cannot decode ASTC, so the encoder is unverified." >&2
            exit 1
        fi
        failed=1
    fi
done

if (( failed )); then
    echo >&2
    echo "FAIL: the platform's ASTC decoder did not reproduce the encoder's input." >&2
    exit 1
fi

echo
echo "PASS: every fixture survived a round trip through the platform's own ASTC decoder."
