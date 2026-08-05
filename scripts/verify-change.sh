#!/usr/bin/env bash
# ============================================================
# Local verification entry point.
# Location: scripts/verify-change.sh
#
# Answers one question: has this change been verified for every target it
# touches. Section 7.4 of the four-platform delivery design is the reason it
# exists -- host `cargo check`, `cargo test` and `cargo clippy` skip
# `cfg(target_os = "android")` code entirely, so a green host run is evidence
# about the portable tree and nothing else. Three Android compile errors rode
# this branch for several sessions on exactly that gap, on the touch path, on
# session teardown and on the permission gate, while every host run stayed green.
#
# What it does:
#   1. audits the module walk the target selection depends on, and stops if any
#      crate source file is unreachable -- an unreached file has unknown
#      conditions, and guessing "portable" is how the gap reopens;
#   2. asks scripts/lib/verification_targets.py which targets the changed files
#      need, and why;
#   3. runs the host suites, always;
#   4. runs the target builds it knows how to run;
#   5. prints one verdict line per target, and fails when a required target was
#      not proven -- including when the toolchain for it is simply absent. A skip
#      there would reproduce the exact failure this script exists to prevent.
#
# Usage:
#   scripts/verify-change.sh [--base <ref>] [--plan-only] [--abi <abi>]
#
#   --base <ref>  compare against this ref's merge base with HEAD.
#                 Default `master`: the scope a pull request would gate, which is
#                 the honest default for a branch that is never pushed.
#                 `--base HEAD` narrows to the working tree alone.
#   --plan-only   report the plan and exit; run nothing.
#   --abi <abi>   Android ABI for the target builds (default arm64-v8a).
#
# The verdict block is meant to be copied into the ledger. "Any change touching
# conditional code names the target build that compiled it" is a specification
# requirement, and a sentence nobody can produce cheaply does not get written.
# ============================================================

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BASE="master"
PLAN_ONLY=false
ABI="arm64-v8a"

print_info()    { printf '\033[0;36m[verify] %s\033[0m\n' "$*"; }
print_success() { printf '\033[0;32m[verify] %s\033[0m\n' "$*"; }
print_error()   { printf '\033[0;31m[verify] %s\033[0m\n' "$*" >&2; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --base)      shift; [[ $# -gt 0 ]] || { print_error "--base requires a ref"; exit 2; }; BASE="$1" ;;
        --base=*)    BASE="${1#*=}" ;;
        --plan-only) PLAN_ONLY=true ;;
        --abi)       shift; [[ $# -gt 0 ]] || { print_error "--abi requires a value"; exit 2; }; ABI="$1" ;;
        --abi=*)     ABI="${1#*=}" ;;
        --help|-h)   sed -n '2,40p' "${BASH_SOURCE[0]}"; exit 0 ;;
        *)           print_error "unknown argument: $1"; exit 2 ;;
    esac
    shift
done

cd "$ROOT"

case "$ABI" in
    arm64-v8a|x86_64) ;;
    *) print_error "unknown ABI: $ABI (expected arm64-v8a or x86_64)"; exit 2 ;;
esac

# ------------------------------------------------------------
# The module walk has to be complete before its answers mean anything.
# ------------------------------------------------------------
unreached="$(python3 scripts/lib/verification_targets.py --root . --audit)" || true
if [[ -n "$unreached" ]]; then
    print_error "no mod declaration reaches these crate sources, so their platform conditions are unknown:"
    while read -r orphan; do
        printf '  %s\n' "$orphan" >&2
    done <<< "$unreached"
    print_error "fix scripts/lib/verification_targets.py's module walk, or the declaration it cannot parse"
    exit 1
fi

# ------------------------------------------------------------
# Scope
# ------------------------------------------------------------
if ! git rev-parse --verify --quiet "$BASE" > /dev/null; then
    print_error "not a ref: $BASE"
    exit 2
fi
merge_base="$(git merge-base HEAD "$BASE")"

changed="$(
    {
        git diff --name-only "$merge_base" HEAD
        git diff --name-only HEAD
        git ls-files --others --exclude-standard
    } | sort -u
)"
changed_count="$(printf '%s' "$changed" | grep -c . || true)"
scope="$BASE..HEAD plus the working tree ($changed_count files)"
print_info "scope: $scope"

if [[ "$changed_count" -eq 0 ]]; then
    print_info "nothing changed against $BASE"
fi

plan="$(printf '%s\n' "$changed" | python3 scripts/lib/verification_targets.py --root .)"
if [[ -n "$plan" ]]; then
    printf '%s\n' "$plan"
fi

if grep -q '^UNDETERMINED$' <<< "$plan"; then
    print_error "changed sources whose platform conditions could not be determined; see UNDETERMINED above"
    exit 1
fi

if [[ "$PLAN_ONLY" == true ]]; then
    exit 0
fi

# ------------------------------------------------------------
# Verdict accounting
# ------------------------------------------------------------
labels=()
commands=()
verdicts=()

record() {
    labels+=("$1")
    commands+=("$2")
    verdicts+=("$3")
}

run_step() {
    local label="$1" command="$2"
    print_info "$label: $command"
    if ( eval "$command" ); then
        record "$label" "$command" "PASS"
        return 0
    fi
    record "$label" "$command" "FAIL"
    return 1
}

# ------------------------------------------------------------
# Host suites. Always: they are the only evidence for the portable tree, and a
# target build does not run a single test.
# ------------------------------------------------------------
HOST_STEPS=(
    "cd engine && cargo build --workspace --all-targets"
    # Before the suites that depend on it: a broken counting allocator would
    # otherwise surface as an unexplained allocation gate failure downstream.
    "cd engine && cargo test -p migo-alloc-probe"
    "cd engine && cargo test -p migo-contention-probe"
    "cd engine && cargo test -p migo-executor-probe"
    "cd engine && cargo test -p migo-shared --lib"
    "cd engine && cargo test -p migo-io --lib"
    "cd engine && cargo test -p migo-runtime-v8 --lib"
    # Carries the occupancy gate on the shared audio streaming worker, so a suite
    # that ran nowhere would leave that gate as decoration.
    "cd engine && cargo test -p migo-audio --lib"
    "cd engine && cargo test -p migo-graphics --lib"
    "cd engine && cargo test -p migo-core --lib"
    "cd engine && cargo test -p migo-capi --lib"
    "cd engine && cargo test -p migo-platform --lib"
    "cd engine && cargo fmt --all --check"
    "git diff --check"
)

for step in "${HOST_STEPS[@]}"; do
    run_step "host" "$step" || true
done

# ------------------------------------------------------------
# Target builds. A platform with no entry here is reported NOT PROVEN, never
# skipped: `ohos` and `windows` conditional code has no local build on this
# machine, and that is a fact about the evidence, not a detail to swallow.
# ------------------------------------------------------------
target_command() {
    case "$1:$2" in
        android:compile) echo "bash scripts/build-android-so.sh --compile-only $ABI" ;;
        android:link)    echo "bash scripts/build-android-so.sh $ABI" ;;
        *)               echo "" ;;
    esac
}

while read -r keyword platform tier; do
    [[ "$keyword" == "TARGET" ]] || continue
    command="$(target_command "$platform" "$tier")"
    if [[ -z "$command" ]]; then
        record "$platform $tier" "no local build for this target" "NOT PROVEN"
        continue
    fi
    run_step "$platform $tier" "$command" || true
done <<< "$plan"

# ------------------------------------------------------------
# Verdicts
# ------------------------------------------------------------
echo
echo "VERIFIED SCOPE  $scope"
failed=0
for index in "${!labels[@]}"; do
    printf '%-11s %-11s %s\n' "${verdicts[$index]}" "${labels[$index]}" "${commands[$index]}"
    [[ "${verdicts[$index]}" == "PASS" ]] || failed=1
done

if [[ "$failed" -ne 0 ]]; then
    print_error "not verified: see the non-PASS lines above"
    exit 1
fi
print_success "verified for every target this change touches"
