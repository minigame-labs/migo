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
    mapfile -t offenders < <(scan "${macos_scripts[@]}")
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
