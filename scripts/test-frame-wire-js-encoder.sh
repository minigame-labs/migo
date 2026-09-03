#!/usr/bin/env bash
# The JavaScript frame encoder must reproduce the committed corpus exactly.
#
# contracts/frame-wire/wire-v1.md says the format has two implementations and
# that neither is the specification: both are measured against the fixed corpus
# in contracts/frame-wire/golden. For a while that was aspirational. There was
# one encoder, so "the corpus is what the Rust builder produces" was a
# tautology, and the sentence in the contract described a check nobody could run.
#
# THE DRIFT THIS EXISTS TO CATCH is the quiet kind. A JavaScript encoder that
# reads a 128-bit launch nonce through `Number` is correct for every value
# anyone types by hand and wrong for the value a real session uses -- and the
# symptom is a renderer in another process rejecting frames as foreign, on a
# device, with no way to see why. The corpus carries values past 2^53 so that
# mistake fails here instead.
#
# Host-only: node, no device, no Apple toolchain.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TEST="platforms/apple/WebContent/PerformancePlus/test/golden-corpus.test.mjs"
ENCODER="platforms/apple/WebContent/PerformancePlus/src/wire-frame-packet.mjs"

for required in "$TEST" "$ENCODER"; do
    if [[ ! -f "$required" ]]; then
        echo "FAIL: $required is missing; the cross-language corpus check cannot run." >&2
        exit 1
    fi
done

if ! command -v node >/dev/null 2>&1; then
    # Not a pass. An environment without node cannot answer this question, and
    # reporting "skipped" as success is how a check stops being one.
    echo "FAIL: node is not available, so the JavaScript encoder is unverified." >&2
    echo "      Install node (any version with BigInt DataView support) or run this on a host that has it." >&2
    exit 1
fi

# The encoder must not have grown a dependency. It runs inside WebContent next
# to untrusted game code; every import is one more thing inside that boundary,
# and the test harness is the only place allowed to reach the filesystem.
if grep -nE '^\s*import\b' "$ENCODER" >/dev/null 2>&1; then
    echo "FAIL: $ENCODER imports something. It runs in WebContent beside untrusted" >&2
    echo "      content and must stay dependency-free; the test harness does the I/O." >&2
    grep -nE '^\s*import\b' "$ENCODER" >&2
    exit 1
fi

node "$TEST"
