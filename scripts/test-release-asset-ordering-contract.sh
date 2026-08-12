#!/usr/bin/env bash
# `dist/release/SHA256SUMS.txt` is a checksum manifest over "everything currently
# in the release staging directory" (`cd dist/release && sha256sum *`).
#
# It used to cover platforms/android/dist -- the Android job's *build output*
# directory -- which is where its dishonesty came from rather than from the file
# itself: that directory holds internal products the release never ships, and holds
# nothing from the Linux, Windows or OpenHarmony packages. Pointed at the staging
# directory instead, it covers exactly the set this job publishes.
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
# and never executes the workflow or any step's script.
#
# A step is put in scope if the dist path can reach it through any of the
# three channels a static reader can see: the step's `run:` text, its
# `working-directory:` (including one inherited from the job's
# `defaults.run`), or a value in its `env:` map (including the job-level
# `env:` it inherits). Scoping on `run:` text alone was itself
# allow-by-default -- review found `working-directory: <dist>` with a
# relative redirect, and an `env:` entry holding the path used as
# `> "$OUT_DIR/f"`, both dismissed before any per-command scrutiny ran. What
# remains undetectable is a path assembled at runtime from data (read out of
# a file, computed by an earlier step). That boundary is real and is stated
# here rather than papered over: this gate covers the statically visible
# channels, not every conceivable one.
#
# Deny-by-default, not allow-by-default: an earlier version of this gate
# recognized three write shapes (redirection, cp/mv, cd+relative-write) and
# called anything that didn't match one of them "clean". Review found
# `tee file`, `install ... file`, an `env FOO=bar cp ... file` prefix, and a
# `python3 -c "...write..."` one-liner all sailed through as clean -- the
# allowlist of write shapes was necessarily incomplete, and every shape
# nobody had thought of yet was silently safe. Inverted: once a step's `run`
# text mentions the staging directory anywhere (or `cd`s into it), every
# command in that step must be affirmatively recognized as read-only, or the
# gate fails and names the command it could not clear. Being unable to
# classify a command is treated exactly like proof that it writes.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKFLOW="${1:-$ROOT_DIR/.github/workflows/release.yml}"

# A gate whose tool is missing must say so, not silently report the
# invariant as satisfied (or as broken) -- see
# scripts/test-surface-attachment-contract.sh for the same principle applied
# to a missing `rg`. Deliberately does NOT try to `pip install` PyYAML itself:
# Ubuntu 24.04+ enforces PEP 668 (externally-managed-environment), so a bare
# `pip install` here can fail on the very runner this is meant to protect.
# The call site (pr-ci.yml) is responsible for making PyYAML available before
# invoking this script; this script only verifies and reports.
if ! python3 -c "import yaml" >/dev/null 2>&1; then
    echo "Release asset ordering contract could not run: PyYAML is not importable by python3." >&2
    echo "This is a missing tool, NOT a contract violation. Make PyYAML available (the CI step" >&2
    echo "that calls this script is responsible for that) and re-run." >&2
    exit 127
fi

python3 - "$WORKFLOW" <<'PY'
from __future__ import annotations

import re
import shlex
import sys

import yaml

workflow_path = sys.argv[1]
DIST = "dist/release"

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
matches: list[tuple[str, dict, list[dict], int]] = []
for job_name, job in jobs.items():
    steps = (job or {}).get("steps") or []
    for idx, step in enumerate(steps):
        run = step.get("run") if isinstance(step, dict) else None
        if isinstance(run, str) and CHECKSUM_CMD_RE.search(run) and CHECKSUM_TARGET_RE.search(run):
            matches.append((job_name, job or {}, steps, idx))

if len(matches) == 0:
    print(
        "ERROR: no step's `run` block invokes sha256sum while writing SHA256SUMS.txt -- "
        f"expected exactly one in {workflow_path}",
        file=sys.stderr,
    )
    sys.exit(1)

if len(matches) > 1:
    where = ", ".join(
        f"{jn}:{(steps[idx] or {}).get('name', f'<step {idx}>')}"
        for jn, _job, steps, idx in matches
    )
    print(
        "ERROR: more than one step's `run` block invokes sha256sum while writing "
        f"SHA256SUMS.txt ({where}) -- this gate cannot tell which is authoritative",
        file=sys.stderr,
    )
    sys.exit(1)

job_name, job, steps, checksum_idx = matches[0]
checksum_step_name = (steps[checksum_idx] or {}).get("name", f"<step {checksum_idx}>")

# --- Narrow, explicit allowlist of read-only shapes -------------------------
#
# Anything not recognized below is treated as a write (fail closed). This is
# deliberately small: it exists to let genuinely read-only shell plumbing
# (an existence-check loop, an `if`/`for` control structure, a diagnostic
# `echo` to stderr) pass, not to anticipate every safe command that could
# ever be written. Extend it only for a concrete, reviewed case.

# Shell keywords/builtins that never touch the filesystem by themselves.
SAFE_LEADING_TOKENS = {
    "then", "else", "elif", "fi", "do", "done", "in", "esac", ":",
    "true", "false", "exit", "return", "cd",
}
# Commands that are read-only UNLESS they carry an output redirection; their
# redirection targets (if any) still have to be proven to land outside DIST.
SAFE_IF_REDIRECTS_ARE_SAFE = {"echo", "printf"}
# The POSIX test command, in both spellings; same redirection caveat.
READONLY_TEST_COMMANDS = {"[", "test"}

IF_PREFIX_RE = re.compile(r"^(if|elif|while|until)\s+(.*)$")
FOR_HEADER_RE = re.compile(r"^for\s+\w+\s+in\b")
CD_RE = re.compile(r"^cd\s+(.+)$")


def logical_lines(run_text: str) -> list[str]:
    """Join backslash line-continuations the way a shell would, so a
    multi-line `for f in \\ ... \\ ; do` becomes one line to classify."""
    joined: list[str] = []
    buf = ""
    for raw_line in run_text.splitlines():
        line = (buf + " " + raw_line) if buf else raw_line
        stripped = line.rstrip()
        if stripped.endswith("\\") and not stripped.endswith("\\\\"):
            buf = stripped[:-1]
            continue
        buf = ""
        joined.append(line)
    if buf:
        joined.append(buf)
    return joined


def split_top_level(line: str, seps=(";", "&&", "||", "|", "&")) -> list[str]:
    """Split a logical shell line into commands at top-level `;`, `&&`,
    `||`, `|`, `&`, respecting simple quoting and leaving fd-duplication
    redirects like `>&1` / `2>&1` intact (a bare `&` there is not a
    backgrounding operator)."""
    chunks: list[str] = []
    buf = ""
    i = 0
    n = len(line)
    in_squote = False
    in_dquote = False
    seps_sorted = sorted(seps, key=len, reverse=True)
    while i < n:
        c = line[i]
        if in_squote:
            buf += c
            if c == "'":
                in_squote = False
            i += 1
            continue
        if in_dquote:
            buf += c
            if c == '"' and line[i - 1] != "\\":
                in_dquote = False
            i += 1
            continue
        if c == "'":
            in_squote = True
            buf += c
            i += 1
            continue
        if c == '"':
            in_dquote = True
            buf += c
            i += 1
            continue
        if c == "&" and i > 0 and line[i - 1] == ">":
            buf += c
            i += 1
            continue
        matched = next((sep for sep in seps_sorted if line.startswith(sep, i)), None)
        if matched:
            chunks.append(buf)
            buf = ""
            i += len(matched)
            continue
        buf += c
        i += 1
    chunks.append(buf)
    return [c.strip() for c in chunks if c.strip() != ""]


def split_shell_words(chunk: str):
    try:
        return shlex.split(chunk)
    except ValueError:
        return None


def redirect_targets(chunk: str) -> list[tuple[str, str]]:
    return [(op, target) for op, _q, target in re.findall(r"(>>?)\s*(\"?)([^\s\"|&;]+)", chunk)]


def redirect_is_safe(target: str, cwd_is_dist: bool) -> bool:
    if target.startswith("&") or target.isdigit():
        return True  # fd duplication (`>&2`), not a filesystem write
    if DIST in target:
        return False
    if "$" in target or "{{" in target:
        return False  # unresolvable -- conservative: not provably safe
    if cwd_is_dist and not target.startswith("/"):
        return False
    return True


def is_safe_chunk(chunk: str, cwd_is_dist: bool) -> tuple[bool, str | None]:
    """Return (True, None) if this single command is affirmatively provable
    as read-only; otherwise (False, reason). No catch-all 'else clean' path:
    anything not matched by the allowlist below falls through to the final
    `return False`."""
    chunk = chunk.strip()
    if not chunk or chunk.startswith("#"):
        return True, None

    m = IF_PREFIX_RE.match(chunk)
    if m:
        chunk = m.group(2).strip()

    if FOR_HEADER_RE.match(chunk):
        return True, None

    words = split_shell_words(chunk)
    if words is None:
        return False, f"could not tokenize `{chunk}` to classify it"
    if not words:
        return True, None

    # A bare `NAME=value` with nothing after it assigns a shell variable and
    # executes nothing. `NAME=value some_command ...` is multiple tokens and
    # falls through instead -- it runs `some_command`, which must itself be
    # cleared.
    if len(words) == 1 and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*=.*", words[0]):
        return True, None

    head = words[0]

    if head in SAFE_LEADING_TOKENS or head in READONLY_TEST_COMMANDS or head in SAFE_IF_REDIRECTS_ARE_SAFE:
        for op, target in redirect_targets(chunk):
            if not redirect_is_safe(target, cwd_is_dist):
                return False, f"`{chunk}` redirects ({op}) into `{target}`"
        return True, None

    return False, f"`{head}` is not a recognized read-only command (`{chunk}`)"


def analyze_run(run_text: str, cwd_is_dist: bool = False) -> tuple[str, str]:
    """Return ('clean' | 'writes' | 'undetermined', reason).

    Called only for steps the caller has already placed in scope, so there is
    deliberately no early return on "DIST is absent from run_text": the path
    can reach a step through `working-directory` or `env` instead of through
    the shell text, and returning clean on those would put the scrutiny below
    behind an allow-by-default filter.

    Every command in the step must clear is_safe_chunk(); the first one that
    doesn't determines the verdict. 'undetermined' (an unresolvable `cd`
    target) is handled identically to 'writes' by the caller -- both fail
    the gate.

    `cwd_is_dist` seeds the working directory, so a step whose
    `working-directory` is the dist directory starts out with relative
    redirects already counting as writes into it.
    """
    for logical in logical_lines(run_text):
        for chunk in split_top_level(logical):
            candidate = chunk
            m = IF_PREFIX_RE.match(candidate)
            if m:
                candidate = m.group(2).strip()
            cd_match = CD_RE.match(candidate)
            if cd_match:
                target_raw = cd_match.group(1).strip()
                target = target_raw.strip("\"'").rstrip("/")
                if target in (DIST, f"./{DIST}"):
                    cwd_is_dist = True
                elif "$" in target or "{{" in target:
                    return (
                        "undetermined",
                        f"`cd {target_raw}` targets an unresolvable path, and this step "
                        f"otherwise mentions {DIST}",
                    )
                else:
                    cwd_is_dist = False
                continue

            ok, reason = is_safe_chunk(chunk, cwd_is_dist)
            if not ok:
                return "writes", reason

    return "clean", ""


def working_directory_of(mapping) -> str:
    value = (mapping or {}).get("working-directory")
    return value if isinstance(value, str) else ""


def env_entry_naming_dist(env) -> str:
    """Describe the first env entry whose value names DIST, else ''."""
    if not isinstance(env, dict):
        return ""
    for key, value in env.items():
        if isinstance(value, str) and DIST in value:
            return f"env `{key}` is `{value}`"
    return ""


def normalise_dir(path: str) -> str:
    return path.strip().strip("\"'").rstrip("/")


# Inherited by any step that does not override them.
job_default_wd = working_directory_of(((job or {}).get("defaults") or {}).get("run"))
job_env_reason = env_entry_naming_dist((job or {}).get("env"))

violations: list[tuple[str, str]] = []
undetermined: list[tuple[str, str]] = []

for idx in range(checksum_idx + 1, len(steps)):
    step = steps[idx] or {}
    step_name = step.get("name", f"<step {idx}>")
    run = step.get("run")

    # A step is in scope if the dist path can reach it through ANY statically
    # visible channel, not just the shell text. Scoping on `run` alone was
    # allow-by-default: `working-directory: <dist>` with a relative redirect,
    # or an `env:` value holding the path and a `"$VAR/f"` redirect, both
    # sailed past before any per-command scrutiny happened.
    step_wd = working_directory_of(step)
    effective_wd = step_wd or job_default_wd
    wd_inherited = not step_wd and bool(job_default_wd)
    cwd_is_dist = bool(effective_wd) and normalise_dir(effective_wd) in (DIST, f"./{DIST}")

    reasons: list[str] = []
    if effective_wd and DIST in effective_wd:
        reasons.append(
            f"working-directory is `{effective_wd}`"
            + (" (inherited from the job's defaults.run)" if wd_inherited else "")
        )
    step_env_reason = env_entry_naming_dist(step.get("env"))
    if step_env_reason:
        reasons.append(step_env_reason)
    elif job_env_reason:
        reasons.append(f"{job_env_reason} (inherited from the job)")
    if isinstance(run, str) and DIST in run:
        reasons.append("its `run` text names the directory")

    if not reasons:
        # Out of scope through every channel this contract can see. A `uses:`
        # step such as the publish action lands here: it reads from dist to
        # upload elsewhere, and names it nowhere this gate can read.
        continue

    if run is None:
        undetermined.append(
            (step_name, f"{reasons[0]}, but the step has no `run` text to scrutinize")
        )
        continue
    if not isinstance(run, str):
        undetermined.append((step_name, "`run` is not a plain string"))
        continue

    verdict, reason = analyze_run(run, cwd_is_dist)
    if verdict == "writes":
        violations.append((step_name, f"{reasons[0]}; {reason}"))
    elif verdict == "undetermined":
        undetermined.append((step_name, f"{reasons[0]}; {reason}"))

if undetermined:
    for name, reason in undetermined:
        print(
            f"ERROR: could not establish that step '{name}' (after '{checksum_step_name}' "
            f"in job '{job_name}') is read-only with respect to {DIST}: {reason} -- being "
            "conservative, treating this as a violation",
            file=sys.stderr,
        )
    sys.exit(1)

if violations:
    for name, reason in violations:
        print(
            f"ERROR: step '{name}' runs after '{checksum_step_name}' in job '{job_name}', "
            f"is in scope for {DIST}, and could not be proven read-only ({reason}) -- "
            "SHA256SUMS.txt is generated before this step runs, so if this step writes into "
            f"{DIST} the published manifest would stop covering everything the release "
            "publishes",
            file=sys.stderr,
        )
    sys.exit(1)

print(
    f"OK: '{checksum_step_name}' (job '{job_name}') is the sole SHA256SUMS.txt writer, and "
    f"every later step in that job is either unrelated to {DIST} or proven read-only against it"
)
PY
