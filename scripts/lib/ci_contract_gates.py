#!/usr/bin/env python3
"""The source-structure contract gates CI runs, read out of the workflow itself.

`scripts/verify-change.sh` had no concept of these. Every one of them is a gate
over source structure -- what a file may import, which resolver an entry point
calls, whether an event's payload keys match its reader -- and they live only in
`.github/workflows/pr-ci.yml`. So the local verifier printed "verified for every
target this change touches" for changes CI rejects, which is the same defect that
put `android-java` in that script: a whole lane it did not know existed.

The list is *derived* rather than restated. A second copy would drift, and the
drift would be silent in the direction that matters -- a gate added to CI and not
here is a gate the local run does not have.

Output: one `<disposition> <command>` line per gate, in workflow order.

`<disposition>` is `run`, `needs:<tool>` when the gate depends on something a
machine may not have (so the caller reports NOT PROVEN rather than a FAIL that
means "this machine"), or `skip:<reason>` for a gate that cannot run from here at
all. Commands keep the environment assignments CI puts in front of them, because
a weaker local invocation of the same script is a quieter gate wearing the same
name.

Usage: ci_contract_gates.py [<repo-root>]
"""

from __future__ import annotations

import pathlib
import re
import sys

WORKFLOW = ".github/workflows/pr-ci.yml"
JOB = "quality-gate"

# A gate whose failure here would mean "the tool is missing", not "the change is
# wrong". Kept short and explicit: an unlisted gate that needs something exotic
# fails loudly, which is the safe direction -- it asks to be looked at, where a
# silent NOT PROVEN would not.
NEEDS = {
    "scripts/test-surface-attachment-contract.sh": "rg",
    "scripts/test-x11-owned-connection-contract.sh": "rg",
    "scripts/test-release-asset-ordering-contract.sh": "pyyaml",
    # These compile the Java half themselves before reading it.
    "scripts/test-android-host-api-contract.sh": "gradlew",
    "scripts/test-camera-frame-jni-contract.sh": "gradlew",
}

# Gates that must not run from inside the verifier.
SKIP = {
    # It runs verify-change.sh against fixture repositories. Running it from
    # within a verify-change.sh invocation would nest the whole gate inside
    # itself, so CI is the only place it belongs.
    "scripts/test-local-verification-contract.sh": "recursive",
}

# Below this, the parse has stopped matching rather than the workflow having
# shrunk, and an empty lane is worse than no lane: it reports success.
MINIMUM_GATES = 15

_JOB_HEADER = re.compile(r"^  (?P<name>[A-Za-z0-9_-]+):\s*$")
_INVOCATION = re.compile(
    r"(?P<env>(?:[A-Z][A-Z0-9_]*=\S+\s+)*)bash\s+(?P<script>scripts/[A-Za-z0-9._/-]+\.sh)"
    r"(?P<args>(?:\s+--?[A-Za-z0-9][^\s#]*)*)"
)


def job_body(workflow_text: str, job: str) -> list[str]:
    """The lines of one job, by indentation: jobs are the only 2-space keys."""
    lines = workflow_text.splitlines()
    start = None
    for index, line in enumerate(lines):
        match = _JOB_HEADER.match(line)
        if match is None:
            continue
        if match.group("name") == job:
            start = index + 1
        elif start is not None:
            return lines[start:index]
    return [] if start is None else lines[start:]


def gates(root: pathlib.Path) -> list[tuple[str, str]]:
    workflow = root / WORKFLOW
    if not workflow.is_file():
        # A tree with no workflow has no CI gates to mirror, which is a fact about
        # the tree rather than a failure to derive anything -- the verification
        # contract's own fixture repositories are exactly this shape. Distinct exit
        # code so the caller can say that instead of reporting a broken lane; a
        # workflow that exists and cannot be parsed still fails.
        print(f"{WORKFLOW} not present in {root}", file=sys.stderr)
        sys.exit(3)

    body = job_body(workflow.read_text(encoding="utf-8"), JOB)
    if not body:
        raise SystemExit(
            f"job `{JOB}` not found in {WORKFLOW}; the contract lane cannot be derived"
        )

    found: list[tuple[str, str]] = []
    seen: set[str] = set()
    for line in body:
        # A commented-out invocation is not an invocation. The gate list has to
        # mean what CI runs, not what its comments mention.
        code = line.split("#", 1)[0]
        for match in _INVOCATION.finditer(code):
            script = match.group("script")
            if not (root / script).is_file():
                continue
            command = (
                f"{match.group('env')}bash {script}{match.group('args')}".strip()
            )
            if command in seen:
                continue
            seen.add(command)
            if script in SKIP:
                found.append((f"skip:{SKIP[script]}", command))
            elif script in NEEDS:
                found.append((f"needs:{NEEDS[script]}", command))
            else:
                found.append(("run", command))

    if len(found) < MINIMUM_GATES:
        raise SystemExit(
            f"derived only {len(found)} contract gate(s) from {WORKFLOW}:{JOB}, "
            f"expected at least {MINIMUM_GATES}; this parse has stopped matching "
            "and the lane would pass vacuously"
        )
    return found


def audit(root: pathlib.Path) -> list[str]:
    """Invocations in the job that this module did not emit, with the reason.

    The completeness question the verification contract asks -- "is CI running a
    source-structure gate the local lane does not have?" -- has to be answered by
    the same parse that builds the lane. Answered by a second grep over the
    workflow it would drift, and it would drift silently: a job the grep also
    matched would read as a missing gate, and a gate this parse stopped seeing
    would read as fine.
    """
    workflow = root / WORKFLOW
    if not workflow.is_file():
        return []
    body = job_body(workflow.read_text(encoding="utf-8"), JOB)
    emitted = {command for _, command in gates(root)}
    unaccounted = []
    for line in body:
        code = line.split("#", 1)[0]
        for match in _INVOCATION.finditer(code):
            script = match.group("script")
            command = (
                f"{match.group('env')}bash {script}{match.group('args')}".strip()
            )
            if command in emitted:
                continue
            reason = (
                "no such file" if not (root / script).is_file() else "not emitted"
            )
            unaccounted.append(f"{command} ({reason})")
    return unaccounted


def main(argv: list[str] | None = None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    if argv and argv[0] == "--audit":
        root = pathlib.Path(argv[1] if len(argv) > 1 else ".").resolve()
        for entry in audit(root):
            print(entry)
        return 0
    root = pathlib.Path(argv[0] if argv else ".").resolve()
    for disposition, command in gates(root):
        print(f"{disposition} {command}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
