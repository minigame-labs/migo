#!/usr/bin/env bash
# A script that runs on macOS must survive bash 3.2 expanding an empty array.
#
# THE DRIFT THIS EXISTS TO CATCH was found by running `scripts/build-apple-sdk.sh`
# for the first time, on 2026-09-05, on a GitHub macOS runner:
#
#   scripts/build-apple-sdk.sh: line 250: cargo_profile_flag[@]: unbound variable
#
# macOS ships bash 3.2 as `/bin/bash` -- it has since 2007, for licensing
# reasons, and GitHub's macOS images are no different. In bash before 4.4,
# `"${arr[@]}"` on an EMPTY array under `set -u` is an unbound-variable error
# rather than an expansion to nothing. On Linux's bash 5 the same line is fine,
# so this class of defect is invisible on every machine this project develops on
# and fatal on the only one that can build an Apple product.
#
# It was not one line. `cargo_profile_flag` is empty for a Debug build and
# `cargo_feature_flags` is empty for the macos-v8 product, so TWO of the three
# documented ways to invoke that script could never have worked -- and nobody
# knew, because the script had never been run at all.
#
# SCOPE IS DERIVED, NOT LISTED. The scripts that matter are the ones a macOS job
# actually invokes, so this reads `.github/workflows/*.yml`, finds every job with
# a `macos-*` runner, and collects the `scripts/*.sh` those jobs run. A list kept
# here would go stale the first time somebody added a step, and a stale scope
# reads exactly like a clean result.
#
# SCOPE FOLLOWS `source`, AND THAT HALF WAS ADDED AFTER NEARLY BEING NEEDED.
# The scope above is "scripts a macOS job NAMES", and a library reached through
# `source` is not named anywhere in a workflow. On 2026-09-05 the Apple ANGLE
# recipe was one commit away from sourcing scripts/lib/v8-patch-apply.sh, which
# uses `mapfile` and `local -A` in its tree-audit half -- both bash 4.0, both
# invisible to this gate, both on the only OS that cannot run them. The recipe
# was changed to carry the twenty lines it actually needed instead, but the hole
# it would have gone through is still a hole, and a scope that stops at the first
# file is the same "covers the side it was designed for" shape this gate's own
# second half exists to answer.
#
# A `source` whose target cannot be resolved is a FAILURE, not a skip. An
# unresolvable path is exactly the case where something bash-4 could be hiding,
# and a scanner that quietly inspects less than it claims is what this file is
# about.
#
# The check is deliberately conservative: it flags an unguarded expansion of any
# array that is assigned `()` anywhere in the file, whether or not the expansion
# happens to sit inside a non-emptiness test. Deciding that statically is not
# possible, and the guarded form costs nothing:
#
#     "${arr[@]}"   ->   ${arr[@]+"${arr[@]}"}
#
# IT ALSO CHECKS THE OTHER HALF, AND THAT SECOND HALF EXISTS BECAUSE THE FIRST
# ONE ALONE LET A DEFECT THROUGH. The first version of this gate covered empty
# array expansion and nothing else. The very next macOS run died in
# `test-apple-performance-rust-closure.sh` -- a script this gate already had in
# scope -- with
#
#     mapfile: command not found
#
# because `mapfile` is a bash 4.0 builtin. A guard that covers one side of the
# thing it is guarding is the failure shape this repository keeps meeting, so
# the check is now the whole documented set of constructs bash 3.2 does not
# have, rather than the one that happened to bite first.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

problems=()
notes=()

# --- scope: the scripts macOS jobs run ---------------------------------------
mapfile -t macos_scripts < <(python3 - <<'PY'
import glob, re, sys
try:
    import yaml
except ModuleNotFoundError:
    sys.stderr.write("PyYAML missing\n"); sys.exit(3)

found = set()
for path in sorted(glob.glob(".github/workflows/*.yml")):
    doc = yaml.safe_load(open(path, encoding="utf-8"))
    for job in (doc or {}).get("jobs", {}).values():
        runner = job.get("runs-on", "")
        if not (isinstance(runner, str) and runner.startswith("macos")):
            continue
        for step in job.get("steps", []) or []:
            run = step.get("run") or ""
            for m in re.finditer(r"(scripts/[A-Za-z0-9_./-]+\.sh)", run):
                found.add(m.group(1))
for s in sorted(found):
    print(s)
PY
) || { echo "FAIL: could not read the workflows to derive scope" >&2; exit 1; }

if (( ${#macos_scripts[@]} == 0 )); then
    problems+=("no macOS job in .github/workflows runs any scripts/*.sh, so this gate
      inspected nothing. Either the Apple lanes stopped running scripts or the
      workflow parsing broke; a clean result here would mean neither.")
else
    notes+=("macOS jobs invoke ${#macos_scripts[@]} script(s): ${macos_scripts[*]}")
fi

# --- scope closure: whatever those scripts source, transitively -----------------
#
# Prints the closure of a set of shell files under `source`/`.`. Paths are
# resolved the way the shell would: `$SCRIPT_DIR` is the sourcing file's own
# directory, `$ROOT`/`$REPO_ROOT`/`$PROJECT_ROOT` are the repository root. A
# target that resolves to nothing prints an `UNRESOLVED` line, which the caller
# turns into a failure.
expand_sources() {
    python3 - "$ROOT" "$@" <<'PY'
import os
import re
import sys

root = os.path.abspath(sys.argv[1])
queue = list(sys.argv[2:])
seen = []
unresolved = []

SOURCE = re.compile(r"""^\s*(?:source|\.)\s+["']?([^"'\s;]+)""")
HEREDOC = re.compile(r"<<-?\s*[\"']?([A-Za-z_][A-Za-z0-9_]*)[\"']?")


def shell_lines(lines):
    """Yield (number, line) for shell code only, skipping heredoc bodies.

    Necessary here and not in the detector below, and the difference is not
    tidiness: `source` is an ordinary Python identifier, and this repository's
    shell scripts embed Python through heredocs constantly. Without this,
    `source = pathlib.Path(...).read_text()` inside one reads as a shell source
    directive whose target is `=`. The detector's tokens -- mapfile, declare -A,
    ${v,,} -- are not Python, so it has never needed this and gains nothing from
    being changed alongside.
    """
    terminator = None
    for number, line in enumerate(lines, 1):
        if terminator is not None:
            if line.strip() == terminator:
                terminator = None
            continue
        stripped = line.lstrip()
        if not stripped.startswith("#"):
            opener = HEREDOC.search(line)
            if opener:
                terminator = opener.group(1)
                yield number, line
                continue
        yield number, line

while queue:
    path = queue.pop(0)
    real = os.path.abspath(path)
    if real in seen:
        continue
    seen.append(real)
    try:
        with open(real, encoding="utf-8", errors="replace") as handle:
            lines = handle.read().splitlines()
    except OSError:
        continue
    here = os.path.dirname(real)
    for number, line in shell_lines(lines):
        if line.lstrip().startswith("#"):
            continue
        match = SOURCE.match(line)
        if not match:
            continue
        target = match.group(1)
        expanded = (target
                    .replace("${SCRIPT_DIR}", here).replace("$SCRIPT_DIR", here)
                    .replace("${BASH_SOURCE[0]%/*}", here)
                    .replace("${ROOT}", root).replace("$ROOT", root)
                    .replace("${REPO_ROOT}", root).replace("$REPO_ROOT", root)
                    .replace("${PROJECT_ROOT}", root).replace("$PROJECT_ROOT", root))
        if "$" in expanded:
            unresolved.append(f"{real}:{number}: sources {target!r}, which this scope "
                              "walk cannot resolve")
            continue
        candidate = expanded if os.path.isabs(expanded) else os.path.join(here, expanded)
        candidate = os.path.abspath(candidate)
        if not os.path.isfile(candidate):
            unresolved.append(f"{real}:{number}: sources {target!r}, which does not exist")
            continue
        queue.append(candidate)

for entry in unresolved:
    print("UNRESOLVED " + entry)
for entry in seen:
    print(entry)
PY
}

# --- the detector -------------------------------------------------------------
# Prints "path:line:name" for every unguarded expansion of a possibly-empty
# array. One implementation, used for the scope above and for the control below.
scan() {
    python3 - "$@" <<'PY'
import re, sys

# Constructs bash 3.2 does not have, with the version that introduced each and
# the replacement. Comment lines are stripped first so a paragraph explaining a
# construct is not reported as using it.
BASH4 = [
    (r"^\s*(?:mapfile|readarray)\s",     "mapfile/readarray (bash 4.0)",
     "while IFS= read -r line; do arr+=(\"$line\"); done < <(...)"),
    (r"^\s*(?:local\s+)?declare\s+-A\b", "associative arrays (bash 4.0)",
     "an indexed array of key=value entries"),
    (r"\$\{[A-Za-z_][A-Za-z0-9_]*(?:\[[^]]*\])?(?:,,|\^\^)",
     "case modification ${v,,} / ${v^^} (bash 4.0)", "tr '[:upper:]' '[:lower:]'"),
    (r"^\s*coproc\b",                    "coproc (bash 4.0)", "a named pipe or a temp file"),
    (r"[^|&]\|&[^|]",                     "|& pipe-with-stderr (bash 4.0)", "2>&1 |"),
    (r"^\s*shopt\s+-s\s+globstar",      "globstar (bash 4.0)", "find"),
]

def strip_comments(src):
    return "\n".join("" if re.match(r"\s*#", line) else line for line in src.splitlines())

for path in sys.argv[1:]:
    try:
        src = open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        continue
    code = strip_comments(src)

    # Array expansion under set -u, which bash only fixed in 4.4.
    #
    # EVERY expansion, not only the arrays this scanner can see assigned `()`.
    # The narrower rule shipped first and missed one within the hour: an array
    # filled by a helper that does `eval "$name=()"` has no literal empty
    # assignment to find, and `for x in "${found[@]}"` then died on the macOS
    # runner exactly as before. Deciding "can this be empty here" statically is
    # not possible, the guarded form costs nothing, and a rule with no exceptions
    # is one nobody has to reason about.
    if re.search(r"^set -[a-z]*u", src, re.M):
        for m in re.finditer(r'(?<!\+)"\$\{([A-Za-z_][A-Za-z0-9_]*)\[@\]\}"', code):
            name = m.group(1)
            line = code[:m.start()].count(chr(10)) + 1
            print(f"{path}:{line}:expands the array {name} unguarded "
                  f"(bash 4.4 fixed that under set -u); write ${{{name}[@]+\"${{{name}[@]}}\"}}")

    for pattern, what, instead in BASH4:
        for m in re.finditer(pattern, code, re.M):
            line = code[:m.start()].count(chr(10)) + 1
            print(f"{path}:{line}:uses {what}; use {instead}")
PY
}

if (( ${#macos_scripts[@]} > 0 )); then
    mapfile -t closure < <(expand_sources "${macos_scripts[@]}")
    scoped=()
    for entry in ${closure[@]+"${closure[@]}"}; do
        case "$entry" in
            UNRESOLVED\ *)
                problems+=("${entry#UNRESOLVED } -- this gate cannot tell whether that
      file is bash 3.2 safe, and an unresolvable source is exactly where
      something bash-4 hides.") ;;
            "") ;;
            *) scoped+=("$entry") ;;
        esac
    done
    if (( ${#scoped[@]} > ${#macos_scripts[@]} )); then
        notes+=("scope closure under source: ${#macos_scripts[@]} named, ${#scoped[@]} scanned")
    fi
    mapfile -t offenders < <(scan ${scoped[@]+"${scoped[@]}"})
    for offender in ${offenders[@]+"${offenders[@]}"}; do
        [[ -n "$offender" ]] || continue
        problems+=("${offender%%:*}:${offender#*:} -- this script runs on macOS, whose
      /bin/bash is 3.2.")
    done
    (( ${#offenders[@]} == 0 )) && notes+=("no bash-4-only construct in the macOS-facing scripts")
fi

# --- the control --------------------------------------------------------------
#
# A file that certainly contains the pattern, run through the same detector. A
# scanner with a wrong regex, or one handed a path that does not exist, reports a
# clean result exactly like a correct one; this repository has published two
# confident false conclusions from unverified detection.
control="$(mktemp -t bash32control.XXXXXX.sh)"
trap 'rm -f "$control"' EXIT
cat > "$control" <<'CONTROL'
#!/usr/bin/env bash
set -euo pipefail
flags=()
echo "${flags[@]}"
mapfile -t lines < /dev/null
CONTROL
# The same question for the scope walk: a file that sources a helper containing
# a bash-4 construct must be reported through the helper. Without this, adding
# `source` following could have been a no-op nobody noticed -- which is the
# defect this gate is named after, one level up.
source_control_dir="$(mktemp -d -t bash32src.XXXXXX)"
trap 'rm -f "$control"; rm -rf "$source_control_dir"' EXIT
mkdir -p "$source_control_dir/lib"
cat > "$source_control_dir/lib/helper.sh" <<'HELPER'
# shellcheck shell=bash
helper_read() {
    mapfile -t lines < /dev/null
}
HELPER
cat > "$source_control_dir/entry.sh" <<'ENTRY'
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib/helper.sh"
ENTRY
mapfile -t source_closure < <(expand_sources "$source_control_dir/entry.sh")
mapfile -t source_hits < <(scan ${source_closure[@]+"${source_closure[@]}"})
if (( ${#source_hits[@]} < 1 )); then
    problems+=("the scope walk did not reach a helper reached through source, so
      following source checks nothing and every clean result above covers only
      the files a workflow happens to name.")
else
    notes+=("control: the scope walk reaches a sourced helper (${#source_hits[@]} finding(s))")
fi

mapfile -t control_hits < <(scan "$control")
if (( ${#control_hits[@]} < 2 )); then
    problems+=("the detector reported ${#control_hits[@]} finding(s) for a file that contains two
      of them -- an unguarded empty-array expansion and a mapfile. The scan is
      broken, so every clean result above is meaningless.")
else
    notes+=("control: the detector finds both planted constructs (${#control_hits[@]} finding(s))")
fi

printf '\n'
for note in ${notes[@]+"${notes[@]}"}; do echo "  - $note"; done
printf '\n'

if (( ${#problems[@]} > 0 )); then
    echo "FAIL: a macOS-facing script would die on bash 3.2." >&2
    printf '\n' >&2
    for problem in ${problems[@]+"${problems[@]}"}; do echo "  * $problem" >&2; done
    printf '\n' >&2
    cat >&2 <<'WHY'
  Why this matters: macOS is the only OS that can build an Apple product, and
  its /bin/bash is 3.2. A script that works on every developer machine here and
  dies there is not caught by any Linux gate -- it is caught the first time
  somebody with a Mac tries to ship, which is the worst possible moment.
WHY
    exit 1
fi

echo "PASS: the macOS-facing scripts survive bash 3.2."
