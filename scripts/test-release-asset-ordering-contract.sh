#!/usr/bin/env bash
# `platforms/android/dist/SHA256SUMS.txt` is a checksum manifest over
# "everything currently in dist" (`cd platforms/android/dist && sha256sum *`).
# It is only trustworthy if nothing writes a new file into that directory
# after it runs -- otherwise the release publishes a file the manifest never
# saw. That bug shipped once already: "Write version metadata" ran after
# "Generate checksums", so the published SHA256SUMS.txt never covered
# version.json.
#
# The fix was to reorder the steps; this gate is what keeps the reorder from
# silently regressing. A comment next to the checksum step cannot fail a
# build, so this repo's convention (see the other scripts/test-*-contract.sh
# gates) is a parsed, executable check instead.
#
# This is source-only: it parses .github/workflows/release.yml with PyYAML
# and inspects each step's `run:` text. It never executes the workflow or any
# step's script.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKFLOW="${1:-$ROOT_DIR/.github/workflows/release.yml}"

# A gate whose tool is missing must say so, not silently report the
# invariant as satisfied (or as broken) -- see
# scripts/test-surface-attachment-contract.sh for the same principle applied
# to a missing `rg`.
if ! python3 -c "import yaml" >/dev/null 2>&1; then
    echo "Release asset ordering contract could not run: PyYAML is not installed for python3." >&2
    echo "This is a missing tool, NOT a contract violation. Install PyYAML (pip install pyyaml) and re-run." >&2
    exit 127
fi

python3 - "$WORKFLOW" <<'PY'
from __future__ import annotations

import re
import shlex
import sys

import yaml

workflow_path = sys.argv[1]
DIST = "platforms/android/dist"

with open(workflow_path, "r", encoding="utf-8") as fh:
    workflow = yaml.safe_load(fh)

jobs = (workflow or {}).get("jobs") or {}
if not jobs:
    print(f"ERROR: {workflow_path} has no jobs -- cannot locate the checksum step", file=sys.stderr)
    sys.exit(1)

CHECKSUM_CMD_RE = re.compile(r"\bsha256sum\b")
CHECKSUM_TARGET_RE = re.compile(r"\bSHA256SUMS\.txt\b")

# Find every step, in every job, whose `run:` text both invokes sha256sum and
# writes SHA256SUMS.txt. There must be exactly one across the workflow: zero
# means the file this gate protects does not exist under the name it expects
# (nothing to anchor the ordering check to); more than one means the gate
# cannot tell which is authoritative. Either way that is itself a finding,
# not something to guess past.
matches: list[tuple[str, list[dict], int]] = []
for job_name, job in jobs.items():
    steps = (job or {}).get("steps") or []
    for idx, step in enumerate(steps):
        run = step.get("run") if isinstance(step, dict) else None
        if isinstance(run, str) and CHECKSUM_CMD_RE.search(run) and CHECKSUM_TARGET_RE.search(run):
            matches.append((job_name, steps, idx))

if len(matches) == 0:
    print(
        "ERROR: no step's `run` block invokes sha256sum while writing SHA256SUMS.txt -- "
        f"expected exactly one in {workflow_path}",
        file=sys.stderr,
    )
    sys.exit(1)

if len(matches) > 1:
    where = ", ".join(
        f"{jn}:{(steps[idx] or {}).get('name', f'<step {idx}>')}" for jn, steps, idx in matches
    )
    print(
        "ERROR: more than one step's `run` block invokes sha256sum while writing "
        f"SHA256SUMS.txt ({where}) -- this gate cannot tell which is authoritative",
        file=sys.stderr,
    )
    sys.exit(1)

job_name, steps, checksum_idx = matches[0]
checksum_step_name = (steps[checksum_idx] or {}).get("name", f"<step {checksum_idx}>")


def split_shell_words(s: str):
    try:
        return shlex.split(s)
    except ValueError:
        return None


def analyze_run(run_text: str):
    """Return ('clean' | 'writes' | 'undetermined', reason) for one step's
    `run:` text, scanning for the three write shapes named in the contract:
    a redirection into DIST, a cp/mv targeting DIST, or a `cd` into DIST
    followed by an output-producing command using a path relative to it.
    Conservative: anything this cannot resolve statically (a variable or
    expression standing in for a path) is 'undetermined', which the caller
    treats the same as a violation.
    """
    cwd_is_dist = False
    for raw_line in run_text.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue

        cd_match = re.match(r"^cd\s+(.+)$", line)
        if cd_match:
            target_raw = cd_match.group(1).split("&&")[0].split(";")[0].strip()
            target = target_raw.strip("\"'").rstrip("/")
            if target in (DIST, f"./{DIST}"):
                cwd_is_dist = True
                continue
            if "$" in target or "{{" in target:
                return (
                    "undetermined",
                    f"`cd {target_raw}` targets a variable/expression path that cannot be "
                    "statically resolved against " + DIST,
                )
            cwd_is_dist = False
            continue

        # Redirection: `> target` / `>> target`, skipping fd-duplication like `2>&1`.
        for op, _quote, target in re.findall(r"(>>?)\s*(\"?)([^\s\"|&;]+)", line):
            if target.startswith("&") or target.isdigit():
                continue
            if DIST in target:
                return ("writes", f"redirects ({op}) into `{target}`")
            if cwd_is_dist:
                if "$" in target or "{{" in target:
                    return (
                        "undetermined",
                        f"redirects into variable path `{target}` while cwd is {DIST} "
                        f"(via a prior `cd {DIST}`)",
                    )
                if not target.startswith("/"):
                    return (
                        "writes",
                        f"redirects ({op}) into `{target}` while cwd is {DIST} "
                        f"(via a prior `cd {DIST}`)",
                    )

        cpmv_match = re.match(r"^(cp|mv)\s+(.*)$", line)
        if cpmv_match:
            cmd, rest = cpmv_match.groups()
            words = split_shell_words(rest)
            if words is None:
                return ("undetermined", f"`{line}` could not be tokenized to find its destination")
            args = [w for w in words if not w.startswith("-")]
            if not args:
                return ("undetermined", f"`{line}` has no discernible destination argument")
            dest = args[-1]
            if DIST in dest:
                return ("writes", f"`{cmd}` targets `{dest}`")
            if "$" in dest or "{{" in dest:
                return (
                    "undetermined",
                    f"`{cmd}` destination `{dest}` is a variable/expression that cannot be "
                    "statically resolved",
                )
            if cwd_is_dist and not dest.startswith("/"):
                return (
                    "writes",
                    f"`{cmd}` targets `{dest}` while cwd is {DIST} (via a prior `cd {DIST}`)",
                )

    return ("clean", "")


violations: list[tuple[str, str]] = []
undetermined: list[tuple[str, str]] = []

for idx in range(checksum_idx + 1, len(steps)):
    step = steps[idx] or {}
    step_name = step.get("name", f"<step {idx}>")
    run = step.get("run")
    if run is None:
        # No shell text for this text-based contract to read (e.g. a `uses:`
        # step like the publish action, which reads from dist to upload
        # elsewhere but has no run body that could write into it). Not
        # flagged -- there is nothing here to find a write in.
        continue
    if not isinstance(run, str):
        undetermined.append((step_name, "`run` is not a plain string"))
        continue

    verdict, reason = analyze_run(run)
    if verdict == "writes":
        violations.append((step_name, reason))
    elif verdict == "undetermined":
        undetermined.append((step_name, reason))

if undetermined:
    for name, reason in undetermined:
        print(
            f"ERROR: cannot determine whether step '{name}' (after '{checksum_step_name}' "
            f"in job '{job_name}') writes into {DIST}: {reason} -- being conservative, "
            "treating this as a violation",
            file=sys.stderr,
        )
    sys.exit(1)

if violations:
    for name, reason in violations:
        print(
            f"ERROR: step '{name}' runs after '{checksum_step_name}' in job '{job_name}' and "
            f"writes into {DIST} ({reason}) -- SHA256SUMS.txt is generated before this step "
            "runs, so the published manifest would stop covering everything the release "
            "publishes",
            file=sys.stderr,
        )
    sys.exit(1)

print(
    f"OK: '{checksum_step_name}' (job '{job_name}') is the sole SHA256SUMS.txt writer, and no "
    f"later step in that job writes into {DIST}"
)
PY
