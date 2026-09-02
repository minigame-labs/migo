#!/usr/bin/env bash
# =============================================================================
# Contract: the Apple profile policy is one document, the Swift enums mirror it
# exactly, and no failure is left without a next-Session outcome.
#
# Three drifts this is aimed at, each of which is silent:
#
#   1. A REASON CODE ADDED IN ONE PLACE. The reason travels from the resolver
#      into telemetry; a Swift case with no policy entry is a value nobody can
#      interpret when it shows up in a report, and a policy entry with no Swift
#      case is a reason the resolver cannot actually emit. Both compile. The
#      comparison is derived from both files, never from a list kept here --
#      a hand-kept third copy would just be a third thing to drift.
#
#   2. A LANE NAMED THAT DOES NOT EXIST. The policy points at lanes defined in
#      deployment-floor.json. A typo there selects nothing and reads like a
#      configuration choice.
#
#   3. A FAILURE WITH NO NEXT SESSION. The whole point of the two-column
#      failure table is that a running Session cannot change lane, so every
#      failure needs a separate answer for the Session after it. An entry
#      missing that column looks complete and leaves the recovery path
#      undefined -- which in practice means "whatever the resolver happens to
#      pick", discovered on a device.
#
# Fails closed: unreadable or unparsable inputs are errors, and a comparison
# that finds nothing to compare is an error too.
# =============================================================================
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

POLICY="$REPO_ROOT/contracts/apple/profile-policy.json"
FLOOR="$REPO_ROOT/contracts/apple/deployment-floor.json"
PROFILE_SWIFT="$REPO_ROOT/platforms/apple/Sources/MigoAppleCore/MigoRuntimeProfile.swift"

err()  { printf '\033[0;31m[apple-policy] %s\033[0m\n' "$*" >&2; }
ok()   { printf '\033[0;32m[apple-policy] %s\033[0m\n' "$*"; }
info() { printf '\033[0;36m[apple-policy] %s\033[0m\n' "$*"; }

for required in "$POLICY" "$FLOOR" "$PROFILE_SWIFT"; do
    if [ ! -f "$required" ]; then
        err "missing input: ${required#$REPO_ROOT/}"
        exit 1
    fi
done

report="$(python3 - "$POLICY" "$FLOOR" "$PROFILE_SWIFT" <<'PY'
import json
import re
import sys

policy_path, floor_path, swift_path = sys.argv[1:4]

with open(policy_path, encoding="utf-8") as handle:
    policy = json.load(handle)
with open(floor_path, encoding="utf-8") as handle:
    floor = json.load(handle)
swift = open(swift_path, encoding="utf-8").read()

problems = []
notes = []

# --- reason codes, derived from both sides -------------------------------
declared = set(policy.get("reason_codes") or [])
if not declared:
    problems.append("the policy declares no reason codes")

# `case name = "value"` or a bare `case name` inside MigoProfileReason.
reason_block = re.search(
    r"enum\s+MigoProfileReason\s*:\s*String[^{]*\{(.*?)\n\}", swift, re.S
)
if reason_block is None:
    problems.append("MigoProfileReason is not a String enum in the Swift mirror")
    mirrored = set()
else:
    mirrored = set(re.findall(r"case\s+([A-Za-z0-9_]+)", reason_block.group(1)))

if not mirrored and reason_block is not None:
    problems.append("MigoProfileReason declares no cases")

for missing in sorted(declared - mirrored):
    problems.append(f"reason {missing!r} is in the policy with no Swift case")
for extra in sorted(mirrored - declared):
    problems.append(f"reason {extra!r} is a Swift case with no policy entry")
notes.append(f"reason codes compared: {len(declared)}")

# --- lanes must exist in the floor contract ------------------------------
lanes = set((floor.get("lanes") or {}).keys())
if not lanes:
    problems.append("the deployment floor contract declares no lanes")

referenced = set()
for name, tier in (policy.get("device_tiers") or {}).items():
    if name.startswith("_"):
        continue
    lane = tier.get("lane")
    if not lane:
        problems.append(f"device tier {name} names no lane")
        continue
    referenced.add(lane)
    if "memory_budget_bytes" not in tier:
        problems.append(f"device tier {name} has no memory budget")

for name, failure in (policy.get("failures") or {}).items():
    if name.startswith("_"):
        continue
    for column in ("current_session", "next_session", "reason_code"):
        if not failure.get(column):
            problems.append(f"failure {name!r} has no {column}")
    reason = failure.get("reason_code")
    if reason and reason not in declared:
        problems.append(f"failure {name!r} cites unknown reason {reason!r}")
    following = failure.get("next_session", "")
    for lane in lanes:
        if lane in following:
            referenced.add(lane)

for unknown in sorted(referenced - lanes):
    problems.append(f"policy references lane {unknown!r}, which the floor contract does not define")
notes.append(f"lanes referenced: {len(referenced)} of {len(lanes)} defined")

# --- every tier reachable, every failure answered ------------------------
tiers = [name for name in (policy.get("device_tiers") or {}) if not name.startswith("_")]
if len(tiers) < 2:
    problems.append("fewer than two device tiers; the tier split is what keeps the floor low")
notes.append(f"device tiers: {len(tiers)}")

failures = [name for name in (policy.get("failures") or {}) if not name.startswith("_")]
if not failures:
    problems.append("the policy answers no failures")
notes.append(f"failures with a next-Session outcome: {len(failures)}")

steps = policy.get("decision_order") or []
expected = list(range(1, len(steps) + 1))
if [entry.get("step") for entry in steps] != expected:
    problems.append("decision_order steps are not 1..N in order")
notes.append(f"decision steps: {len(steps)}")

for note in notes:
    print(f"NOTE\t{note}")
for problem in problems:
    print(f"PROBLEM\t{problem}")
sys.exit(1 if problems else 0)
PY
)"
status=$?

printf '%s\n' "$report" | while IFS=$'\t' read -r kind text; do
    case "$kind" in
        NOTE)    info "$text" ;;
        PROBLEM) err "$text" ;;
    esac
done

if [ "$status" -ne 0 ]; then
    err "Apple profile policy contract: FAIL"
    exit 1
fi

ok "Apple profile policy contract: PASS"
exit 0
