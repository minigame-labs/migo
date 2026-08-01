#!/usr/bin/env bash
# The prescreen scanner must not under-report.
#
# `scripts/prescreen-game.sh` answers "which APIs does this bundle need, and
# does this build publish them". Its dangerous failure is silent under-reporting:
# a bundle reaches the namespace through an alias or a computed key, the scanner
# does not follow it, and the report comes back saying everything is supported.
# That is the one wrong answer that costs a customer, so the scanner's job is
# either to resolve an access or to say out loud that it could not.
#
# The fixture in `tools/api-surface/fixture/` exercises each access form once,
# against a checked-in synthetic surface rather than a live dump -- the scanner's
# logic is what is under test, and binding it to a player run would make this
# slow, machine-dependent, and prone to changing whenever the runtime does.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE="$ROOT_DIR/tools/api-surface/fixture"
SCANNER="$ROOT_DIR/scripts/prescreen-game.sh"

for required in "$FIXTURE/bundle" "$FIXTURE/surface.json" "$SCANNER"; do
    [[ -e "$required" ]] || { echo "ERROR: missing $required" >&2; exit 1; }
done

report="$(bash "$SCANNER" "$FIXTURE/bundle" --surface "$FIXTURE/surface.json" 2>/dev/null)"

[[ -n "$report" ]] || { echo "FAIL: scanner produced no report" >&2; exit 1; }

failures=0
check() { # description, then a grep -E pattern the report must contain
    if grep -qE "$2" <<<"$report"; then
        printf '  ok    %s\n' "$1"
    else
        printf '  FAIL  %s\n' "$1" >&2
        failures=$((failures + 1))
    fi
}
refute() {
    if grep -qE "$2" <<<"$report"; then
        printf '  FAIL  %s\n' "$1" >&2
        failures=$((failures + 1))
    else
        printf '  ok    %s\n' "$1"
    fi
}

echo "prescreen scanner contract:"

# Nine distinct names across every access form the fixture uses. If this number
# drops, some form stopped being followed -- which is under-reporting.
check "resolves all nine referenced names" '\| .wx\.\*. names referenced \| 9 \|'

# The alias form (`const W = wx; W.reportMonitor()`) is the one a naive scanner
# misses, and minified bundles are full of it.
check "follows an alias binding" '\| of those, published by this build \| 7 \|'

# Present on wx, absent from this build: the only bucket that is a real gap.
check "reports a genuine gap" 'wx\.createOffScreenCanvas'
check "counts exactly one genuine gap" '\| \*\*not published, and wx has it\*\* \| \*\*1\*\* \|'

# Absent from wx too: a different thing entirely, and must not be counted as a
# gap in this runtime.
check "separates non-wx names" 'thisApiDoesNotExistAnywhere'
check "counts exactly one non-wx name" '\| not published, and not a wx API either \| 1 \|'

# The unresolvable bucket is what keeps the counts honest.
check "reports both unresolvable sites" '\| \*\*sites this scanner cannot resolve\*\* \| \*\*2\*\* \|'
check "names the computed access" 'computed member access'
check "names the reflection" 'for-in over the namespace'
check "says the counts are a lower bound" 'lower bound'

# With unresolvable sites present, the report must never claim completeness.
refute "does not claim complete coverage while sites are unresolved" \
    'Every namespace access in this bundle is a literal name'

# The caveats are the difference between a report and a claim.
check "states it is not a compatibility verdict" 'Not a compatibility verdict'
check "states Linux cannot answer it" 'Linux player has no device services'

if [[ $failures -gt 0 ]]; then
    echo "FAIL: prescreen scanner contract ($failures check(s))" >&2
    exit 1
fi

echo "PASS: prescreen scanner contract"
