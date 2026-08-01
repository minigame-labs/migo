#!/usr/bin/env bash
# The host-bridge holder is reachable from content, and its user list may not grow.
#
# `globalThis[Symbol.for('Migo.hostBridge')]` holds every `_internal*` hook the
# host calls into. `Symbol.for` reads the *global* symbol registry, so content
# retrieves the same symbol and reaches all of them -- measured against a real
# runtime: 78 hooks, and content can forge a rewarded-video completion through
# `_internalOnAdEvent`.
#
# That does not weaken the reward invariant (the host stays authoritative for
# what it reports to its ad network; content forging its own callback deceives
# only itself). It matters where the JS context holds more than one trust
# domain -- a publisher-injected anti-cheat or analytics prelude expecting to
# observe real host events -- and it weakens the auditable-boundary claim.
#
# Closing it means host callbacks stop travelling as eval'd source that has to
# name a holder content can also name. The design is in CLAUDE.md §8; the
# migration is all-or-nothing, because a call site left behind evaluates against
# a holder that has been deleted and fails **silently**, on the one channel that
# carries every async result -- login, payment, location, camera, keyboard,
# scanCode, share, subpackage.
#
# Until that lands this gate does the one useful thing available: it pins the
# set of call sites so the debt cannot grow while nobody is looking, and gives
# the migration a number that has to reach zero.
#
# When migrating: the count only ever goes down. Update EXPECTED as you go, and
# delete the holder from globalThis **last** -- the ordering is the whole
# safety property.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Call sites that build JS source naming the content-reachable holder.
# `js_escape.rs` is excluded: it *defines* the constant and the builder.
EXPECTED_HOST_RS=10
EXPECTED_INBOUND_RS=4

count_in() {
    grep -c "HOST_BRIDGE_EXPR" "$1" 2>/dev/null || echo 0
}

host_rs="$ROOT_DIR/engine/crates/core/src/runtime/host.rs"
inbound_rs="$ROOT_DIR/engine/crates/platform/src/android/jni/inbound.rs"
escape_rs="$ROOT_DIR/engine/crates/shared/src/js_escape.rs"

for required in "$host_rs" "$inbound_rs" "$escape_rs"; do
    [[ -f "$required" ]] || { echo "ERROR: $required not found" >&2; exit 1; }
done

# Uses, not the `use` line that imports the constant.
host_count=$(( $(count_in "$host_rs") - 1 ))
inbound_count=$(( $(count_in "$inbound_rs") - 1 ))

failures=0

report() {
    printf '  %-52s %s\n' "$1" "$2"
}

echo "host-bridge channel inventory:"
report "core/src/runtime/host.rs" "$host_count (pinned $EXPECTED_HOST_RS)"
report "platform/src/android/jni/inbound.rs" "$inbound_count (pinned $EXPECTED_INBOUND_RS)"

if (( host_count > EXPECTED_HOST_RS )); then
    echo "FAIL: host.rs gained a host-bridge call site ($host_count > $EXPECTED_HOST_RS)" >&2
    echo "      Every one of these is reachable from content. Route the new callback" >&2
    echo "      through the retained-handle channel instead -- see CLAUDE.md §8." >&2
    failures=1
fi
if (( inbound_count > EXPECTED_INBOUND_RS )); then
    echo "FAIL: inbound.rs gained a host-bridge call site ($inbound_count > $EXPECTED_INBOUND_RS)" >&2
    failures=1
fi

# Going down is the point, but the pin has to follow or the gate stops meaning
# anything: a pin above the real count silently permits re-growth back up to it.
if (( host_count < EXPECTED_HOST_RS || inbound_count < EXPECTED_INBOUND_RS )); then
    echo "FAIL: the count went down without the pin following it" >&2
    echo "      host.rs=$host_count inbound.rs=$inbound_count" >&2
    echo "      Lower EXPECTED_HOST_RS/EXPECTED_INBOUND_RS in this script to match." >&2
    echo "      A pin left above the real count permits growing back to it unnoticed." >&2
    failures=1
fi

# Anti-vacuity: if the constant is renamed, every count above reads zero and the
# gate congratulates itself on a surface it can no longer see.
#
# Matches the declaration with comments stripped, not the name anywhere in the
# file: doc comments here discuss `HOST_BRIDGE_EXPR` at length, and a grep that
# sees those reports the constant as present after it has been renamed away.
if ! sed 's|//.*||' "$escape_rs" | grep -qE '^[[:space:]]*pub const HOST_BRIDGE_EXPR[[:space:]]*:'; then
    echo "FAIL: HOST_BRIDGE_EXPR is not defined in js_escape.rs" >&2
    echo "      If the channel was replaced, delete this gate along with it." >&2
    echo "      If it was renamed, this gate has been counting nothing." >&2
    failures=1
fi

if (( failures )); then
    exit 1
fi

total=$(( host_count + inbound_count ))
echo "PASS: host-bridge channel inventory unchanged ($total call sites to migrate)"
