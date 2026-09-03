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
import copy
from collections import Counter
import re
import sys

policy_path, floor_path, swift_path = sys.argv[1:4]

problems = []
notes = []

def read_json(path, label):
    try:
        with open(path, encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, json.JSONDecodeError) as exc:
        problems.append(f"cannot read {label}: {exc}")
        return {}


def read_text(path, label):
    try:
        with open(path, encoding="utf-8") as handle:
            return handle.read()
    except OSError as exc:
        problems.append(f"cannot read {label}: {exc}")
        return ""


policy = read_json(policy_path, "policy JSON")
floor = read_json(floor_path, "deployment floor JSON")
swift = read_text(swift_path, "Swift profile mirror")
if not isinstance(policy, dict):
    problems.append("policy JSON must be an object")
    policy = {}
if not isinstance(floor, dict):
    problems.append("deployment floor JSON must be an object")
    floor = {}

# One mapping drives the JSON contract and the Swift comparison. These are
# runtime inputs, not byte values that can be frozen into the binary.
memory_policy_sources = {
    "content_cap": "verified_content_manifest",
    "host_cap": "host_configuration",
    "measured_device_safe_cap": "versioned_device_lab_policy",
    "available_memory_headroom": "fresh_available_memory_advisory",
    "emergency_reserve": "versioned_memory_policy",
}
memory_policy_fields = tuple(memory_policy_sources) + (
    "reservation_before_alloc",
    "requery_before_large_reservation",
)
dynamic_memory_fields = memory_policy_fields[:5]


def normalized_key(key):
    return re.sub(r"[^a-z0-9]", "", str(key).lower())


def is_number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def is_numeric_text(value):
    return isinstance(value, str) and bool(re.fullmatch(
        r"\s*\d+(?:\.\d+)?\s*(?:gib|gb|mib|mb|bytes?)?\s*", value, re.I
    ))


def positive_jetsam_limit_claim(value):
    """Reject numeric positive claims, while allowing truthful negation prose."""
    if not isinstance(value, str):
        return False
    number = r"(?:\b\d+(?:\.\d+)?\s*(?:gib|gb|mib|mb|bytes?)?\b|\b\d{7,}\b)"
    phrase = r"(?:jetsam\s+(?:memory\s+)?(?:limit|cap)|(?:limit|cap)\s+(?:is\s+)?(?:the\s+)?jetsam)"
    for match in re.finditer(rf"(?:{number}.{{0,48}}{phrase}|{phrase}.{{0,48}}{number})", value, re.I):
        context = value[max(0, match.start() - 20):match.end() + 20]
        if not re.search(r"\b(?:not|no|never|without|isn't|isnt|does\s+not|is\s+not)\b", context, re.I):
            return True
    return False


def clause_has_local_negation(clause, target):
    prefix = clause[:target]
    return bool(re.search(
        r"\b(?:do\s+not|does\s+not|did\s+not|don't|doesn't|didn't|never|must\s+not|cannot|can't|no)\b",
        prefix,
        re.I,
    ))


def fixed_memory_literal(value):
    if not isinstance(value, str):
        return False
    if re.fullmatch(r"\s*\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\s*", value):
        return True
    for match in re.finditer(r"\d[\d_]*", value):
        if len(match.group(0).replace("_", "")) >= 7:
            return True
    if re.search(r"\d+(?:\.\d+)?[eE][+-]?\d+", value):
        return True
    return bool(re.search(
        r"\d[\d_]*(?:\.\d+)?\s*(?:bytes?|kib|mib|gib|kb|mb|gb)",
        value,
        re.I,
    ))


def dynamic_descriptor_diagnostics(document):
    diagnostics = []
    memory = document.get("memory_policy") if isinstance(document, dict) else None
    if not isinstance(memory, dict):
        return diagnostics

    def visit(value, path):
        if is_number(value):
            diagnostics.append(f"forbidden numeric constant in dynamic descriptor at {'.'.join(path)}")
        elif isinstance(value, str) and fixed_memory_literal(value):
            diagnostics.append(f"forbidden fixed memory literal in dynamic descriptor at {'.'.join(path)}")
        elif isinstance(value, dict):
            for key, child in value.items():
                visit(child, path + (str(key),))
        elif isinstance(value, list):
            for index, child in enumerate(value):
                visit(child, path + (str(index),))

    for field in dynamic_memory_fields:
        if field in memory:
            visit(memory[field], ("memory_policy", field))
    return diagnostics


def forbidden_form_diagnostics(document):
    """Find semantic fixed-cap forms without banning harmless explanatory prose."""
    diagnostics = []

    def add(message, path):
        diagnostics.append(f"{message} at {'.'.join(path) or '<document>'}")

    def visit(value, path=()):
        if isinstance(value, dict):
            for key, child in value.items():
                normalized = normalized_key(key)
                if normalized.endswith("memorybudgetbytes") or normalized.endswith("memorybudget"):
                    add("forbidden fixed runtime budget field 'memory_budget_bytes'", path + (str(key),))
                elif "osprocavailablememory" in normalized:
                    add("forbidden cached 'os_proc_available_memory' field", path + (str(key),))
                elif (
                    ("ram" in normalized or "memory" in normalized)
                    and any(term in normalized for term in ("percent", "percentage", "pct", "ratio", "fraction"))
                ):
                    add("forbidden device/physical RAM percentage field", path + (str(key),))
                elif (
                    ("ram" in normalized or "memory" in normalized)
                    and any(term in normalized for term in ("budget", "cap"))
                ):
                    add("forbidden fixed runtime memory cap field", path + (str(key),))
                elif (
                    "jetsam" in normalized
                    and any(term in normalized for term in ("limit", "cap", "budget", "bytes"))
                    and (is_number(child) or is_numeric_text(child))
                ):
                    add("forbidden numeric jetsam-limit field", path + (str(key),))
                visit(child, path + (str(key),))
        elif isinstance(value, list):
            for index, child in enumerate(value):
                visit(child, path + (str(index),))
        elif isinstance(value, str):
            for clause in re.split(r"[.;!?\n]+", value):
                lowered = clause.lower()
                cached_target = lowered.find("os_proc_available_memory")
                if (
                    re.search(r"\b(?:cache|cached|caching)\b", clause, re.I)
                    and cached_target >= 0
                    and not clause_has_local_negation(clause, cached_target)
                ):
                    add("forbidden cached available-memory advisory claim", path)
                    break
                device_match = re.search(r"\b(?:device|physical)\b", clause, re.I)
                ram_match = re.search(r"\b(?:ram|memory)\b", clause, re.I)
                percentage_match = re.search(r"%|\b(?:percent|percentage|pct|ratio|fraction)\b", clause, re.I)
                target_positions = [match.start() for match in (device_match, ram_match) if match is not None]
                if (
                    device_match is not None
                    and ram_match is not None
                    and percentage_match is not None
                    and not clause_has_local_negation(clause, min(target_positions))
                ):
                    add("forbidden device/physical RAM percentage claim", path)
                    break
                if positive_jetsam_limit_claim(clause):
                    add("forbidden numeric jetsam-limit claim", path)
                    break

    visit(document)
    return diagnostics


def tier_structure_diagnostics(document):
    diagnostics = []
    tiers = document.get("device_tiers") if isinstance(document, dict) else None
    if not isinstance(tiers, dict):
        return diagnostics
    allowed = {"lane", "conditions", "note"}
    for name, tier in tiers.items():
        if str(name).startswith("_") or not isinstance(tier, dict):
            continue
        for extra in sorted(key for key in tier if not str(key).startswith("_") and key not in allowed):
            diagnostics.append(f"device tier has unsupported field {extra!r}; tiers only select admission and asset quality")
    return diagnostics


def swift_fixed_literal_diagnostics(document):
    diagnostics = []
    document = strip_swift_comments(document)
    for match in re.finditer(r"(?<![A-Za-z0-9])\d[\d_]*(?![A-Za-z0-9])", document):
        if len(match.group(0).replace("_", "")) >= 7:
            diagnostics.append(
                f"forbidden fixed memory byte literal in Swift mirror at {match.group(0)}"
            )
    for match in re.finditer(r"(?<![A-Za-z0-9])0x[0-9A-Fa-f_]+(?![A-Za-z0-9])", document):
        if int(match.group(0)[2:].replace("_", ""), 16) > 0xFFFF:
            diagnostics.append(
                f"forbidden fixed memory byte literal in Swift mirror at {match.group(0)}"
            )
    return diagnostics


def strip_swift_comments(document):
    result = []
    state = "normal"
    block_depth = 0
    escaped = False
    index = 0
    while index < len(document):
        char = document[index]
        following = document[index + 1] if index + 1 < len(document) else ""
        if state == "normal":
            if char == "/" and following == "/":
                result.extend((" ", " "))
                index += 2
                state = "line_comment"
                continue
            if char == "/" and following == "*":
                result.extend((" ", " "))
                index += 2
                state = "block_comment"
                block_depth = 1
                continue
            result.append(char)
            if char == '"':
                state = "string"
                escaped = False
        elif state == "string":
            result.append(char)
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                state = "normal"
        elif state == "line_comment":
            if char == "\n":
                result.append(char)
                state = "normal"
            else:
                result.append(" ")
        else:
            if char == "/" and following == "*":
                result.extend((" ", " "))
                index += 2
                block_depth += 1
                continue
            if char == "*" and following == "/":
                result.extend((" ", " "))
                index += 2
                block_depth -= 1
                if block_depth == 0:
                    state = "normal"
                continue
            result.append("\n" if char == "\n" else " ")
        index += 1
    if state == "block_comment":
        result.append("\n__UNTERMINATED_SWIFT_BLOCK_COMMENT__\n")
    return "".join(result)


def swift_memory_policy_body(document):
    stripped = strip_swift_comments(document)
    if "__UNTERMINATED_SWIFT_BLOCK_COMMENT__" in stripped:
        return None
    declaration = re.search(
        r"enum\s+MigoMemoryPolicyField\s*:\s*String\s*,?\s*Sendable[^\{]*\{",
        stripped,
    )
    if declaration is None:
        return None
    opening = declaration.end() - 1
    depth = 1
    index = opening + 1
    while index < len(stripped) and depth:
        if stripped[index] == "{":
            depth += 1
        elif stripped[index] == "}":
            depth -= 1
        index += 1
    if depth:
        return None
    return stripped[opening + 1:index - 1]


def swift_memory_policy_diagnostics(document):
    body = swift_memory_policy_body(document)
    if body is None:
        return ["MigoMemoryPolicyField is not a String, Sendable enum in the Swift mirror"]

    expected = {
        "contentCap": "content_cap",
        "hostCap": "host_cap",
        "measuredDeviceSafeCap": "measured_device_safe_cap",
        "availableMemoryHeadroom": "available_memory_headroom",
        "emergencyReserve": "emergency_reserve",
        "reservationBeforeAlloc": "reservation_before_alloc",
        "requeryBeforeLargeReservation": "requery_before_large_reservation",
    }
    diagnostics = []
    cases = []
    case_line = re.compile(r"^\s*case\b.*$", re.M)
    declaration_pattern = re.compile(
        r"^\s*case\s+([A-Za-z_][A-Za-z0-9_]*)(?:\s*=\s*(\"(?:\\.|[^\"\\])*\"))?\s*$"
    )
    for match in case_line.finditer(body):
        declaration = match.group(0)
        parsed = declaration_pattern.match(declaration)
        if parsed is None:
            diagnostics.append("Swift memory policy case declaration is not parseable")
            continue
        name, raw_literal = parsed.groups()
        try:
            raw = json.loads(raw_literal) if raw_literal is not None else None
        except json.JSONDecodeError:
            diagnostics.append(f"Swift memory policy case {name!r} has an invalid String raw value")
            raw = None
        cases.append((name, raw))

    names = [name for name, _ in cases]
    raws = [raw for _, raw in cases if raw is not None]
    for name in sorted({name for name in names if names.count(name) > 1}):
        diagnostics.append(f"Swift memory policy enum repeats case name {name!r}")
    for raw in sorted({raw for raw in raws if raws.count(raw) > 1}):
        diagnostics.append(f"Swift memory policy enum repeats raw value {raw!r}")

    mirrored = {name: raw for name, raw in cases}
    for name, raw in expected.items():
        if name not in mirrored:
            diagnostics.append(f"memory policy field {raw!r} has no explicit Swift case")
        elif mirrored[name] is None:
            diagnostics.append(f"memory policy case {name!r} must have an explicit String raw value")
        elif mirrored[name] != raw:
            diagnostics.append(f"memory policy case {name!r} must use raw value {raw!r}")
    for name, raw in cases:
        if raw is None:
            diagnostics.append(f"memory policy case {name!r} must have an explicit String raw value")
        if name not in expected:
            diagnostics.append(f"Swift memory policy case {name!r} is not in the policy contract")
        elif raw is not None and raw not in expected.values():
            diagnostics.append(f"Swift memory policy raw value {raw!r} is not in the policy contract")
    return diagnostics


def validate_memory_contract(document, swift_document):
    diagnostics = []
    memory = document.get("memory_policy") if isinstance(document, dict) else None
    if not isinstance(memory, dict):
        diagnostics.append("memory_policy must be an object")
    else:
        actual_fields = {key for key in memory if not str(key).startswith("_")}
        required_fields = set(memory_policy_fields)
        for missing in sorted(required_fields - actual_fields):
            diagnostics.append(f"memory_policy is missing required field {missing!r}")
        for extra in sorted(actual_fields - required_fields):
            diagnostics.append(f"memory_policy has unknown field {extra!r}")
        for field in dynamic_memory_fields:
            descriptor = memory.get(field)
            if not isinstance(descriptor, dict):
                diagnostics.append(f"memory_policy field {field!r} must be a runtime descriptor")
                continue
            descriptor_fields = {key for key in descriptor if not str(key).startswith("_")}
            for extra in sorted(descriptor_fields - {"source", "value_kind", "note"}):
                diagnostics.append(f"memory_policy descriptor {field!r} has unsupported field {extra!r}")
            for required in ("source", "value_kind", "note"):
                if required not in descriptor:
                    diagnostics.append(f"memory_policy descriptor {field!r} is missing field {required!r}")
            expected_source = memory_policy_sources[field]
            if descriptor.get("source") != expected_source:
                diagnostics.append(f"memory_policy field {field!r} must use source {expected_source!r}")
            if descriptor.get("value_kind") != "runtime_bytes":
                diagnostics.append(f"memory_policy field {field!r} must use value_kind 'runtime_bytes'")
            if not isinstance(descriptor.get("note"), str) or not descriptor.get("note"):
                diagnostics.append(f"memory_policy field {field!r} must explain its runtime input")
        for field in ("reservation_before_alloc", "requery_before_large_reservation"):
            if memory.get(field) is not True:
                diagnostics.append(f"memory_policy invariant {field!r} must be true")

    diagnostics.extend(forbidden_form_diagnostics(document))
    diagnostics.extend(dynamic_descriptor_diagnostics(document))
    diagnostics.extend(tier_structure_diagnostics(document))
    diagnostics.extend(swift_memory_policy_diagnostics(swift_document))
    diagnostics.extend(swift_fixed_literal_diagnostics(swift_document))
    return diagnostics


problems.extend(validate_memory_contract(policy, swift))

# --- forbidden-form injection self-tests ---------------------------------
# Each test uses a deep copy and checks its own diagnostic, so an unrelated
# pre-existing policy failure cannot make a self-test pass accidentally.
injections = (
    (
        "fixed runtime budget",
        lambda candidate: inject_tier_field(candidate, "memory_budget_bytes", 1),
        "forbidden fixed runtime budget field 'memory_budget_bytes'",
    ),
    (
        "cached available memory",
        lambda candidate: candidate.setdefault("memory_policy", {}).update(os_proc_available_memory=1),
        "forbidden cached 'os_proc_available_memory' field",
    ),
    (
        "device RAM percentage",
        lambda candidate: inject_tier_field(candidate, "device_ram_percentage", 0.5),
        "forbidden device/physical RAM percentage field",
    ),
    (
        "numeric jetsam limit",
        lambda candidate: candidate.setdefault("memory_policy", {}).update(jetsam_limit_bytes=2147483648),
        "forbidden numeric jetsam-limit field",
    ),
    (
        "cached available-memory advisory string",
        lambda candidate: candidate["memory_policy"].update(_comment="cache os_proc_available_memory() as a maximum"),
        "forbidden cached available-memory advisory claim",
    ),
    (
        "device RAM percentage claim",
        lambda candidate: candidate["device_tiers"]["T0"].update(note="50% of device RAM"),
        "forbidden device/physical RAM percentage claim",
    ),
    (
        "dynamic descriptor decimal literal",
        lambda candidate: candidate["memory_policy"]["content_cap"].update(note="1073741824"),
        "forbidden fixed memory literal in dynamic descriptor",
    ),
    (
        "dynamic descriptor unit literal",
        lambda candidate: candidate["memory_policy"]["content_cap"].update(note="1 GiB"),
        "forbidden fixed memory literal in dynamic descriptor",
    ),
    (
        "tier runtime cap field",
        lambda candidate: candidate["device_tiers"]["T0"].update(runtime_cap="1 GiB"),
        "device tier has unsupported field 'runtime_cap'",
    ),
)


def inject_tier_field(candidate, key, value):
    tiers = candidate.get("device_tiers")
    if not isinstance(tiers, dict):
        tiers = {}
        candidate["device_tiers"] = tiers
    tier = tiers.get("T0")
    if not isinstance(tier, dict):
        tier = {}
        tiers["T0"] = tier
    tier[key] = value


swift_injections = (
    (
        "Swift decimal fixed byte literal",
        lambda source: source + "\nprivate let fixedRuntimeCapBytes = 1_073_741_824\n",
        "forbidden fixed memory byte literal in Swift mirror",
    ),
    (
        "Swift hexadecimal fixed byte literal",
        lambda source: source + "\nprivate let fixedRuntimeCapBytes = 0x40000000\n",
        "forbidden fixed memory byte literal in Swift mirror",
    ),
)

def differential_memory_injection(name, inject_policy=None, inject_swift=None, expected=None, forbidden=None):
    baseline = validate_memory_contract(copy.deepcopy(policy), swift)
    candidate_policy = copy.deepcopy(policy)
    candidate_swift = swift
    if inject_policy is not None:
        inject_policy(candidate_policy)
    if inject_swift is not None:
        candidate_swift = inject_swift(candidate_swift)
    candidate = validate_memory_contract(candidate_policy, candidate_swift)
    added = Counter(candidate) - Counter(baseline)
    if expected is not None and not any(expected in diagnostic for diagnostic in added.elements()):
        problems.append(f"injection {name!r} did not add diagnostic: {expected}")
    elif forbidden is not None and any(forbidden in diagnostic for diagnostic in added.elements()):
        problems.append(f"injection {name!r} added forbidden diagnostic: {forbidden}")
    else:
        notes.append(f"differential injection test: {name} detected")


if isinstance(policy, dict):
    for name, inject, expected in injections:
        differential_memory_injection(name, inject_policy=inject, expected=expected)
for name, inject, expected in swift_injections:
    differential_memory_injection(name, inject_swift=inject, expected=expected)


differential_memory_injection(
    "commented required Swift case",
    inject_swift=lambda source: source.replace(
        '    case contentCap = "content_cap"\n',
        '    // case contentCap = "content_cap"\n',
    ),
    expected="memory policy field 'content_cap' has no explicit Swift case",
)
differential_memory_injection(
    "nested block-commented required Swift case",
    inject_swift=lambda source: source.replace(
        '    case contentCap = "content_cap"\n',
        '    /* outer comment starts\n       /* nested comment */\n       case contentCap = "content_cap"\n    */\n',
    ),
    expected="memory policy field 'content_cap' has no explicit Swift case",
)
differential_memory_injection(
    "bare rogue Swift case",
    inject_swift=lambda source: source.replace(
        '    case contentCap = "content_cap"\n',
        '    case contentCap = "content_cap"\n    case rogueRuntimeCap\n',
    ),
    expected="memory policy case 'rogueRuntimeCap' must have an explicit String raw value",
)
differential_memory_injection(
    "duplicate Swift case name",
    inject_swift=lambda source: source.replace(
        '    case contentCap = "content_cap"\n',
        '    case contentCap = "content_cap"\n    case contentCap = "content_cap_duplicate"\n',
    ),
    expected="Swift memory policy enum repeats case name 'contentCap'",
)
differential_memory_injection(
    "duplicate Swift raw value",
    inject_swift=lambda source: source.replace(
        '    case contentCap = "content_cap"\n',
        '    case contentCap = "content_cap"\n    case rogueRuntimeCap = "content_cap"\n',
    ),
    expected="Swift memory policy enum repeats raw value 'content_cap'",
)
differential_memory_injection(
    "descriptor extra schema field",
    inject_policy=lambda candidate: candidate["memory_policy"]["content_cap"].update(fixed_cap="1e9"),
    expected="memory_policy descriptor 'content_cap' has unsupported field 'fixed_cap'",
)
differential_memory_injection(
    "negative RAM prohibition",
    inject_policy=lambda candidate: candidate["device_tiers"]["T0"].update(note="Never allocate 50% of device RAM."),
    forbidden="forbidden device/physical RAM percentage claim",
)
differential_memory_injection(
    "unrelated negation plus cached advisory claim",
    inject_policy=lambda candidate: candidate["memory_policy"].update(_comment="Do not use stale samples; cache os_proc_available_memory() as a maximum."),
    expected="forbidden cached available-memory advisory claim",
)
differential_memory_injection(
    "negative cached advisory prohibition",
    inject_policy=lambda candidate: candidate["memory_policy"].update(_comment="Do not cache os_proc_available_memory() as a maximum."),
    forbidden="forbidden cached available-memory advisory claim",
)
differential_memory_injection(
    "unrelated negation plus RAM claim",
    inject_policy=lambda candidate: candidate["device_tiers"]["T0"].update(note="Do not use stale samples; allocate 50% of device RAM."),
    expected="forbidden device/physical RAM percentage claim",
)

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
floor_lanes = floor.get("lanes") or {}
if not isinstance(floor_lanes, dict):
    problems.append("the deployment floor lanes must be an object")
    floor_lanes = {}
lanes = set(floor_lanes.keys())
if not lanes:
    problems.append("the deployment floor contract declares no lanes")

device_tiers = policy.get("device_tiers") or {}
if not isinstance(device_tiers, dict):
    problems.append("device_tiers must be an object")
    device_tiers = {}
referenced = set()
for name, tier in device_tiers.items():
    if name.startswith("_"):
        continue
    if not isinstance(tier, dict):
        problems.append(f"device tier {name} is not an object")
        continue
    lane = tier.get("lane")
    if not isinstance(lane, str) or not lane:
        problems.append(f"device tier {name} names no lane")
        continue
    referenced.add(lane)

failures_by_name = policy.get("failures") or {}
if not isinstance(failures_by_name, dict):
    problems.append("failures must be an object")
    failures_by_name = {}
for name, failure in failures_by_name.items():
    if name.startswith("_"):
        continue
    if not isinstance(failure, dict):
        problems.append(f"failure {name!r} is not an object")
        continue
    for column in ("current_session", "next_session", "reason_code"):
        if not isinstance(failure.get(column), str) or not failure.get(column):
            problems.append(f"failure {name!r} has no {column}")
    reason = failure.get("reason_code")
    if isinstance(reason, str) and reason not in declared:
        problems.append(f"failure {name!r} cites unknown reason {reason!r}")
    following = failure.get("next_session", "")
    if isinstance(following, str):
        for lane in lanes:
            if lane in following:
                referenced.add(lane)

for unknown in sorted(referenced - lanes):
    problems.append(f"policy references lane {unknown!r}, which the floor contract does not define")
notes.append(f"lanes referenced: {len(referenced)} of {len(lanes)} defined")

# --- every tier reachable, every failure answered ------------------------
tiers = [name for name in device_tiers if not name.startswith("_")]
if len(tiers) < 2:
    problems.append("fewer than two device tiers; the tier split is what keeps the floor low")
notes.append(f"device tiers: {len(tiers)}")

failures = [name for name in failures_by_name if not name.startswith("_")]
if not failures:
    problems.append("the policy answers no failures")
notes.append(f"failures with a next-Session outcome: {len(failures)}")

steps = policy.get("decision_order") or []
if not isinstance(steps, list):
    problems.append("decision_order must be an array")
    steps = []
else:
    expected = list(range(1, len(steps) + 1))
    if any(not isinstance(entry, dict) for entry in steps):
        problems.append("decision_order entries must be objects")
    elif [entry.get("step") for entry in steps] != expected:
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
