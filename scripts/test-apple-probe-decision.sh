#!/usr/bin/env bash
# The G0 decision tool must refuse the data that cannot decide anything.
#
# Worker versus Window, the transport, the frame clock and the WebView host
# shape are unresolved by design, and the design says so in prose. Prose does
# not fail a build. This gate runs the tool that turns probe measurements into
# a decision, against generated matrices that each break one evidence rule, and
# checks that the tool says no.
#
# THE DRIFT THIS EXISTS TO CATCH is the one that has already happened to this
# project once, in another form: a rule that lives only in a document is a rule
# that gets satisfied by whoever is writing the report. "The simulator worked",
# "we ran the interesting half of the matrix" and "the means differed" all look
# like evidence in a summary. Each of them is a case in the suite below.
#
# It also runs on Linux, months before the first Mac. That is deliberate: the
# tool's rules are checkable without a device even though its input is not, and
# the alternative is discovering on lab day that the decision procedure has a
# bug in it.
#
# Host-only: python3, no device, no Apple toolchain.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TOOL="tools/apple-probe-decision/decide.py"
TESTS="tools/apple-probe-decision/tests/test_decide.py"
SCHEMA="contracts/apple/performance-probe.schema.json"

for required in "$TOOL" "$TESTS" "$SCHEMA"; do
    if [[ ! -f "$required" ]]; then
        echo "FAIL: $required is missing; the G0 decision procedure cannot be checked." >&2
        exit 1
    fi
done

if ! command -v python3 >/dev/null 2>&1; then
    echo "FAIL: python3 is not available, so the decision procedure is unverified." >&2
    exit 1
fi

# The schema is the single source of the rules the tool enforces, so a malformed
# one would make every check below vacuous rather than red.
python3 - "$SCHEMA" <<'PY'
import json, sys
schema = json.load(open(sys.argv[1], encoding="utf-8"))
rules = schema["decision_rules"]
missing = [
    key
    for key in (
        "min_samples_per_arm",
        "min_arms_per_variable",
        "simulator_counts_as_evidence",
        "correctness_mismatch_disqualifies_arm",
        "confidence",
        "bootstrap_seed",
    )
    if key not in rules
]
if missing:
    raise SystemExit(f"FAIL: {sys.argv[1]} declares no {', '.join(missing)}")
if rules["simulator_counts_as_evidence"]:
    raise SystemExit(
        "FAIL: the schema admits simulator measurements as evidence. A simulator runs "
        "the host's engine on the host's CPU with the host's memory; it can answer an "
        "ABI question and none of the questions this decision asks."
    )
if rules["min_samples_per_arm"] < 20:
    raise SystemExit(
        f"FAIL: min_samples_per_arm is {rules['min_samples_per_arm']}. Below twenty, the "
        "interval this tool reports is wider than the effect it is looking for."
    )
PY

python3 "$TESTS"
