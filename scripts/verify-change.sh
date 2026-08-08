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
#   4. runs the source-structure contract gates, derived from the workflow that
#      already runs them so the local verdict cannot drift from CI's;
#   5. runs the target builds it knows how to run;
#   6. prints one verdict line per target, and fails when a required target was
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

# ------------------------------------------------------------
# The host suites, declared before anything runs so the script can report them.
#
# `--list-host-crates` exists because the contract needs this list and used to
# recover it by grepping this file for `cargo test -p migo-...`. That coupled a
# check to a spelling: the day the steps stopped carrying the word `cargo`
# themselves, the grep matched nothing and -- under `set -e`, with no `|| true`
# -- the contract died before it could say so. Asking the script is one
# authority instead of a regular expression guessing at one.
# ------------------------------------------------------------
HOST_CARGO_STEPS=(
    "build --workspace --all-targets"
    # Before the suites that depend on it: a broken counting allocator would
    # otherwise surface as an unexplained allocation gate failure downstream.
    "test -p migo-alloc-probe"
    "test -p migo-contention-probe"
    "test -p migo-executor-probe"
    "test -p migo-shared"
    "test -p migo-io --lib"
    "test -p migo-runtime-v8 --lib"
    # Carries the occupancy gate on the shared audio streaming worker, so a suite
    # that ran nowhere would leave that gate as decoration.
    "test -p migo-audio --lib"
    "test -p migo-graphics --lib"
    "test -p migo-core --lib"
    "test -p migo-capi --lib"
    "test -p migo-platform --lib"
    # The Slim product profile, which nothing ran until 2026-08-08. Every crate
    # above declares `default = ["profile-full"]`, so a plain `cargo test` compiles
    # `api-media`, `api-commerce`, `api-connectivity`, `api-sensors` and
    # `api-system` **on** and never builds a single `cfg(not(feature = ...))`
    # branch. The first Slim host run reported 36 failures, and one group of them
    # was a real defect rather than a test gap: the window-resize ingress lived in
    # the `api-connectivity` extension, so no Slim build ever adopted its surface
    # size and every canvas kept the size the window had before a rotation.
    #
    # `runtime-v8` and `core` are the two crates whose capability surface the
    # profile selects. `graphics` takes its profile from `core`, and `capi` and
    # `platform` do not build on the host at all.
    "test -p migo-runtime-v8 --lib --no-default-features --features profile-slim"
    "test -p migo-core --lib --no-default-features --features profile-slim"
    "fmt --all --check"
)

if [[ "${1:-}" == "--list-host-crates" ]]; then
    for step in "${HOST_CARGO_STEPS[@]}"; do
        [[ "$step" =~ -p\ (migo-[a-z0-9-]+) ]] && printf '%s\n' "${BASH_REMATCH[1]}"
    done | sort -u
    exit 0
fi


while [[ $# -gt 0 ]]; do
    case "$1" in
        --base)      shift; [[ $# -gt 0 ]] || { print_error "--base requires a ref"; exit 2; }; BASE="$1" ;;
        --base=*)    BASE="${1#*=}" ;;
        --plan-only) PLAN_ONLY=true ;;
        --abi)       shift; [[ $# -gt 0 ]] || { print_error "--abi requires a value"; exit 2; }; ABI="$1" ;;
        --abi=*)     ABI="${1#*=}" ;;
        --list-host-crates) ;;  # handled above, before any work
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
    if gates="$(python3 "$SCRIPT_DIR/lib/ci_contract_gates.py" "$ROOT" 2>&1)"; then
        printf 'CONTRACT %s gate(s) derived from .github/workflows/pr-ci.yml\n' \
            "$(printf '%s\n' "$gates" | grep -c .)"
        printf '%s\n' "$gates" | sed 's/^/  /'
    else
        printf 'CONTRACT undeterminable: %s\n' "$gates"
    fi
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

# `migo-graphics`, `migo-core`, `migo-capi` and `migo-platform` link Skia, and a
# minimal Linux host cannot build it with a bare `cargo`: it needs the system
# clang rather than the NDK's, the Khronos headers, and the linux-gnu V8
# archive. `dev-test-host.sh` is where this repository already establishes all
# three, so these steps run through it instead of restating any of it here.
#
# **Without this, four of the fourteen host steps failed on an untouched tree**,
# which is the worst state for a verifier to be in: it reports the same red
# whatever the change did, and the only thing it can teach a reader is to stop
# reading it. Measured that way — `--base HEAD` with nothing modified, four
# FAILs — before the routing was added.
HOST_CARGO="cd engine && cargo"
if bash "$ROOT/scripts/dev-test-host.sh" --probe >/dev/null 2>&1; then
    HOST_CARGO="bash scripts/dev-test-host.sh"
else
    # Said out loud rather than absorbed. On a host without the native
    # toolchain the Skia-linked steps below will fail for a reason that has
    # nothing to do with the change under verification, and a reader has to
    # know that is what they are looking at.
    print_info "host: native toolchain unavailable; Skia-linked suites will \
report their environment, not this change"
fi

# Which of those steps this change actually needs. The selector answers with the
# changed packages plus their reverse-dependency closure, so a change to a leaf crate
# still runs the suites of everything that depends on it -- that is where its
# behaviour is observed. A Java-only change now pays for none of them instead of
# sixteen.
#
# Every branch here fails towards running more. `ALL` is what the selector returns for
# a tree cargo cannot describe, for a file under `engine/` belonging to no member, and
# for any path outside `engine/` that is not provably irrelevant; a missing
# `HOSTSUITES` line lands in the same branch. `build --workspace --all-targets` and
# `fmt --all --check` are kept whenever any package is implicated, because several
# workspace members have no suite of their own and that build is the only thing that
# compiles them.
host_selection="ALL"
if host_line="$(grep -m1 '^HOSTSUITES ' <<< "$plan")"; then
    host_selection="${host_line#HOSTSUITES }"
fi

selected_host_steps=()
case "$host_selection" in
    ALL)
        selected_host_steps=("${HOST_CARGO_STEPS[@]}")
        ;;
    NONE)
        # Nothing Rust changed. `fmt` is kept because it is a second and costs
        # nothing, and because a stray unformatted file is worth catching wherever it
        # came from.
        for step in "${HOST_CARGO_STEPS[@]}"; do
            [[ "$step" == fmt* ]] && selected_host_steps+=("$step")
        done
        print_info "host: no workspace member changed; running formatting only"
        ;;
    *)
        for step in "${HOST_CARGO_STEPS[@]}"; do
            if [[ "$step" =~ -p\ (migo-[a-z0-9-]+) ]]; then
                package="${BASH_REMATCH[1]}"
                for candidate in $host_selection; do
                    if [[ "$candidate" == "$package" ]]; then
                        selected_host_steps+=("$step")
                        break
                    fi
                done
            else
                selected_host_steps+=("$step")
            fi
        done
        print_info "host: $((${#selected_host_steps[@]})) of ${#HOST_CARGO_STEPS[@]} \
steps for $host_selection"
        ;;
esac

for step in "${selected_host_steps[@]}"; do
    run_step "host" "$HOST_CARGO $step" || true
done

run_step "host" "git diff --check" || true

# ------------------------------------------------------------
# Source-structure contract gates.
#
# This script had no concept of them, and that is the same defect that put
# `android-java` below, one layer further out. They are gates over structure a
# test cannot reach -- what a crate may depend on, which resolver an entry point
# calls, whether an event's payload keys match its reader -- and they lived only
# in `.github/workflows/pr-ci.yml`. Found by A12's own mutation evidence:
# reverting one ad entry point to a bare handler lookup, which is the exact
# defect that item fixed, left every unit test green and was caught by a contract
# script this script never ran. So the local verdict said "verified for every
# target this change touches" about a change CI rejects.
#
# The list is derived from the workflow rather than restated here. A second copy
# would drift, and it would drift silently in the direction that matters: a gate
# added to CI and not here is a gate the local run does not have.
#
# They run unconditionally, like the host suites. Each is seconds, and keying
# them to changed files means maintaining a file list per gate -- a list to
# forget an entry from, which is how a gate stops covering what it names.
# ------------------------------------------------------------
# Every Gradle build this run drives is offline, for the reason the `android-java`
# lane below gives: this script verifies sources, not the dependency graph, and an
# unconstrained resolve here stalls for tens of minutes. Exported rather than passed,
# because the contract gates are invoked with the command line CI uses -- that parity
# is the point of deriving them -- so the flag cannot live in the command string.
export MIGO_GRADLE_OFFLINE=1

have_tool() {
    case "$1" in
        rg)      command -v rg >/dev/null 2>&1 ;;
        pyyaml)  python3 -c "import yaml" >/dev/null 2>&1 ;;
        gradlew) [[ -x "$ROOT/platforms/android/gradlew" ]] ;;
        *)       return 1 ;;
    esac
}

contract_gates="$(python3 "$SCRIPT_DIR/lib/ci_contract_gates.py" "$ROOT" 2>&1)"
case "$?" in
    0) ;;
    3)  # No workflow in this tree, so there are no CI gates to mirror. Said out
        # loud rather than omitted, because a silent absence is how a lane stops
        # covering anything.
        record "contract" "no .github/workflows/pr-ci.yml in this tree" "CI ONLY"
        contract_gates=""
        ;;
    *)  # The workflow is there and the lane could not be derived from it. That is
        # a failure, not missing evidence: the alternative is a run that silently
        # stops checking a whole class of gate.
        record "contract" "derive gate list from .github/workflows/pr-ci.yml" "FAIL"
        print_error "contract lane: $contract_gates"
        contract_gates=""
        ;;
esac

# Into an array first, and every gate runs with stdin closed. Both matter for the
# same reason: read from a here-string, a gate that consumes stdin -- one of these
# runs `cargo`, which does -- swallows the rest of the list, and the gates after it
# vanish from the verdict without a word. That is the silent under-run this lane
# exists to prevent, and it happened here first: three gates and the `CI ONLY` line
# were missing from a run that reported success.
contract_lane=()
if [[ -n "$contract_gates" ]]; then
    mapfile -t contract_lane <<< "$contract_gates"
fi

for entry in "${contract_lane[@]}"; do
    disposition="${entry%% *}"
    command="${entry#* }"
    [[ -n "$disposition" && -n "$command" ]] || continue
    case "$disposition" in
        run)
            run_step "contract" "$command" </dev/null || true
            ;;
        needs:*)
            tool="${disposition#needs:}"
            if have_tool "$tool"; then
                run_step "contract" "$command" </dev/null || true
            else
                record "contract" "$command" "NOT PROVEN"
            fi
            ;;
        skip:*)
            record "contract" "$command" "CI ONLY"
            ;;
    esac
done

# ------------------------------------------------------------
# Target builds. A platform with no entry here is reported NOT PROVEN, never
# skipped: `ohos` and `windows` conditional code has no local build on this
# machine, and that is a fact about the evidence, not a detail to swallow.
# ------------------------------------------------------------
#
# `android-java` is the Android SDK's other half. It is here because this script
# had no idea Java existed: `platforms/android/**` produced an empty plan, so a
# change to the shipped AAR's own sources ran eleven Rust suites, cross-compiled
# Rust for Android, and printed "verified for every target this change touches"
# without compiling a line of Java. That is the same defect this script was
# written to prevent -- a green run that is evidence about one language and
# silent about another -- one layer out from the `cfg(target_os = "android")`
# gap in its header.
#
# Both product variants, because the Java source set is variant-independent
# while `BuildConfig` capability gating is not: Slim compiles the same files
# with different flags, and a handler behind `MIGO_API_COMMERCE` that only Full
# exercises is exactly the shape this repository has shipped broken before.
target_command() {
    case "$1:$2" in
        android:compile)      echo "bash scripts/build-android-so.sh --compile-only $ABI" ;;
        android:link)         echo "bash scripts/build-android-so.sh $ABI" ;;
        android-java:compile)
            # Probed rather than assumed, so a machine without the Android
            # build reports NOT PROVEN like every other absent target. Running
            # a missing `gradlew` would record FAIL, which says "your change
            # broke this" about a machine that never had the evidence.
            # `--offline` because this lane verifies the sources, not the dependency
            # graph, and without it Gradle tries to refresh its external modules:
            # measured at over twenty minutes against seventeen seconds offline on a
            # machine whose network cannot reach the repositories quickly. A gate that
            # can hang that long is a gate people learn to skip. The failure mode it
            # introduces is loud and names its own cause -- "No cached version
            # available for offline mode" -- unlike a silent stall.
            [[ -x "$ROOT/platforms/android/gradlew" ]] && echo "cd platforms/android && \
./gradlew --quiet --offline :library:testFullDebugUnitTest :library:testSlimDebugUnitTest"
            ;;
        *)                    echo "" ;;
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
    # `CI ONLY` is the one non-PASS verdict that does not fail the run, and
    # the exception is closed: it marks a gate that runs *this script* over
    # fixture repositories, so running it here would nest the gate inside
    # itself. Everything else -- including a target whose toolchain is
    # simply absent -- stays a failure, because unproven is not verified.
    case "${verdicts[$index]}" in
        PASS|"CI ONLY") ;;
        *) failed=1 ;;
    esac
done

if [[ "$failed" -ne 0 ]]; then
    print_error "not verified: see the non-PASS lines above"
    exit 1
fi
print_success "verified for every target this change touches"
