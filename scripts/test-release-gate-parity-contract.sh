#!/usr/bin/env bash
# Every step in pr-ci.yml's quality-gate must also appear in release.yml's
# quality-gate, with the same run body.
#
# The drift this gate exists to prevent was measured: pr-ci had 38 quality-gate
# steps, release had 30, and ten were absent -- including the three that check
# release asset ordering, release version consistency, and artifact timestamp
# reproducibility. Those three gates did not run on the workflow that cuts
# releases. The other seven covered the Android merged-manifest permissions,
# runtime generation fencing, JNI outbound signatures, the OpenHarmony
# newer-sysroot selector, the OpenHarmony API floor declaration, the local
# verification entry point, and Python script syntax. A tag built against the
# clean PR gate could still fail all ten of those properties.
#
# The direction is one-way: release.yml may carry extra steps that make sense
# only at tag time (snapshot freshness, artifact manifest syntax). Each
# exemption states why the PR step is not appropriate for the release workflow.
# A stale exemption -- one that names a PR step that no longer exists -- fails
# the gate, because an exemption that accounts for nothing looks like it does.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT_DIR" <<'PY'
from __future__ import annotations

import pathlib
import sys
import textwrap

try:
    import yaml
except ModuleNotFoundError:
    print(
        "ERROR: PyYAML is not installed; run: pip install --break-system-packages pyyaml",
        file=sys.stderr,
    )
    sys.exit(1)

root = pathlib.Path(sys.argv[1]).resolve()
pr_path = root / ".github" / "workflows" / "pr-ci.yml"
rel_path = root / ".github" / "workflows" / "release.yml"

for path in (pr_path, rel_path):
    if not path.is_file():
        print(f"ERROR: {path.relative_to(root)} not found", file=sys.stderr)
        sys.exit(1)

pr_doc = yaml.safe_load(pr_path.read_text())
rel_doc = yaml.safe_load(rel_path.read_text())

# Fail closed: a missing job is not a pass.
pr_job = (pr_doc.get("jobs") or {}).get("quality-gate")
rel_job = (rel_doc.get("jobs") or {}).get("quality-gate")

if pr_job is None:
    print(
        "ERROR: pr-ci.yml has no quality-gate job; this gate cannot check anything",
        file=sys.stderr,
    )
    sys.exit(1)
if rel_job is None:
    print(
        "ERROR: release.yml has no quality-gate job; this gate cannot check anything",
        file=sys.stderr,
    )
    sys.exit(1)

pr_steps = pr_job.get("steps") or []
rel_steps = rel_job.get("steps") or []

# Fail closed: an empty step list is a parse that stopped matching, not a pass.
if not pr_steps:
    print(
        "ERROR: pr-ci.yml quality-gate has no steps; the gate would pass vacuously",
        file=sys.stderr,
    )
    sys.exit(1)
if not rel_steps:
    print(
        "ERROR: release.yml quality-gate has no steps; the gate would pass vacuously",
        file=sys.stderr,
    )
    sys.exit(1)

# Steps that exist in pr-ci but legitimately do not belong in the release
# workflow. Each entry states why. A stale exemption (the pr-ci step it names
# no longer exists) fails the gate -- an exemption that accounts for nothing
# looks like it does, and can hide a real absence behind it.
#
# The goal of Part 1 of the drift repair was to make this dict empty. Before
# reaching for a new exemption, ask whether the step can simply be added to
# release.yml instead.
EXEMPT: dict[str, str] = {
    # No exemptions. Every pr-ci quality-gate step now runs in release too.
}

pr_by_name = {s["name"]: s for s in pr_steps if "name" in s}
rel_by_name = {s["name"]: s for s in rel_steps if "name" in s}

pr_names = list(pr_by_name)

errors: list[str] = []

# Stale exemptions first -- an exemption whose subject vanished is a hole.
for name, reason in EXEMPT.items():
    if name not in pr_by_name:
        errors.append(
            f"STALE EXEMPTION: '{name}' is exempted as {reason!r} but no longer "
            "exists in pr-ci.yml's quality-gate; drop the exemption rather than "
            "leaving one that accounts for nothing"
        )

# For every pr-ci step, check presence and run-body equality in release.
missing: list[str] = []
body_drift: list[str] = []

for name in pr_names:
    if name in EXEMPT:
        continue
    if name not in rel_by_name:
        missing.append(name)
        continue
    # run body comparison: absent `run` on both sides is also equal (uses:)
    pr_run = pr_by_name[name].get("run")
    rel_run = rel_by_name[name].get("run")
    if pr_run != rel_run:
        body_drift.append(
            f"'{name}': run body differs\n"
            + "  pr-ci:   "
            + textwrap.shorten(repr(pr_run), 120)
            + "\n"
            + "  release: "
            + textwrap.shorten(repr(rel_run), 120)
        )

for name in missing:
    errors.append(
        f"MISSING FROM RELEASE: '{name}' is in pr-ci.yml quality-gate but absent "
        "from release.yml quality-gate -- the tag gate is narrower than the PR gate"
    )
for drift in body_drift:
    errors.append(
        f"RUN BODY DRIFT: {drift} -- a step that kept its name and lost its command "
        "is the same bug as a missing step"
    )

if errors:
    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    print(
        f"Release gate parity contract: FAIL ({len(errors)} violation(s); "
        f"pr-ci has {len(pr_names)} quality-gate step(s), "
        f"release has {len(rel_by_name)} quality-gate step(s))",
        file=sys.stderr,
    )
    sys.exit(1)

checked = len(pr_names) - len(EXEMPT)
print(
    f"Release gate parity contract: PASS ({checked} pr-ci step(s) verified present "
    f"and matching in release.yml; {len(EXEMPT)} legitimately release-exempt)"
)
PY
