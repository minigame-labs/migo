#!/usr/bin/env bash
# =============================================================================
# Contract: every Apple deployment target in the tree derives from one file,
# and no lane claims to be available below the floor the binary can even load
# on.
#
# There are two different numbers here and the whole gate exists because they
# look like one:
#
#   * the deployment target -- the lowest OS the shipped binary loads on;
#   * a lane minimum -- the lowest OS on which one runtime lane is eligible.
#
# The superseded v3 design collapsed them. It raised the iOS deployment target
# to 17.0 to buy a single memory-budget tier, and paid roughly 2.4 points of
# user coverage for a build simplification, because "Performance+ needs 15.2"
# had been allowed to mean "the product needs 15.2". Separating them is what
# lets the floor sit at what the toolchain permits while the fast lane still
# gates on the thing it actually requires.
#
# WHAT THIS CHECKS, AND WHY EACH CHECK IS NOT THE OTHERS.
#
#   1. The contract parses and every version in it is well formed. A gate whose
#      source of truth is unreadable must fail, not skip.
#   2. Every derived declaration matches the contract exactly. Today that is
#      Package.swift's platforms list, MigoDeploymentFloor.swift's constants and
#      the build script's exported Xcode variables.
#   3. No file anywhere else in the tree sets an Apple deployment target. This
#      is the check that survives: 1 and 2 only see the consumers that exist
#      today, and the failure mode is always the consumer added later that
#      carries its own copy. The sweep is derived from the tree, not from a
#      list kept here, for the same reason.
#   4. Every lane minimum is at or above its platform's floor. A lane minimum
#      below the floor is unreachable configuration that still reads as
#      supported.
#   5. With --artifacts DIR, LC_BUILD_VERSION minos in the built Mach-O files.
#      This is the only check that verifies the product rather than the
#      declaration, and it is the one that matters: an Android floor was
#      declared correctly for API 26 while the shipped library carried a strong
#      reference to an API 29 symbol and failed to load. Declarations were all
#      consistent. The artifact was wrong.
#
# Fails closed. An unreadable contract, a missing tool in artifact mode, or a
# sweep that matches nothing at all are errors, not passes.
# =============================================================================
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"

CONTRACT="$REPO_ROOT/contracts/apple/deployment-floor.json"
PACKAGE_SWIFT="$REPO_ROOT/platforms/apple/Package.swift"
# The engine-free package carries its own platforms list because it is a package
# in its own right, built and tested on every PR without the xcframework. Two
# manifests mean two places the floor can be wrong, which is why both are checked
# rather than only the shipping one.
CORE_PACKAGE_SWIFT="$REPO_ROOT/platforms/apple/core/Package.swift"
FLOOR_SWIFT="$REPO_ROOT/platforms/apple/core/Sources/MigoAppleCore/MigoDeploymentFloor.swift"
BUILD_SCRIPT="$REPO_ROOT/scripts/build-apple-sdk.sh"

ARTIFACT_DIR=""

err()  { printf '\033[0;31m[apple-floor] %s\033[0m\n' "$*" >&2; }
ok()   { printf '\033[0;32m[apple-floor] %s\033[0m\n' "$*"; }
info() { printf '\033[0;36m[apple-floor] %s\033[0m\n' "$*"; }

failures=0
fail() { err "$*"; failures=$((failures + 1)); }

usage() {
    cat <<'USAGE'
usage: test-apple-deployment-floor-contract.sh [--artifacts DIR]

  --artifacts DIR   Additionally read LC_BUILD_VERSION from every Mach-O under
                    DIR and require its minos to equal the declared floor.
                    Requires vtool or otool, so in practice a macOS runner.
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --artifacts) ARTIFACT_DIR="${2:-}"; shift 2 ;;
        -h|--help)   usage; exit 0 ;;
        *)           err "unknown argument: $1"; usage >&2; exit 2 ;;
    esac
done

# ---------------------------------------------------------------------------
# 1. The contract itself
# ---------------------------------------------------------------------------

if [ ! -f "$CONTRACT" ]; then
    err "missing contract: $CONTRACT"
    exit 1
fi

if ! contract_values="$(python3 - "$CONTRACT" <<'PY'
import json
import re
import sys

path = sys.argv[1]
with open(path, encoding="utf-8") as handle:
    contract = json.load(handle)

version = re.compile(r"^\d+\.\d+$")
problems = []

platforms = contract.get("platforms") or {}
if not platforms:
    problems.append("contract declares no platforms")

for name, entry in sorted(platforms.items()):
    target = entry.get("deployment_target", "")
    if not version.match(str(target)):
        problems.append(f"{name}.deployment_target is not MAJOR.MINOR: {target!r}")
    if not entry.get("reason"):
        problems.append(f"{name} has no reason; a floor without one cannot be reviewed")

lanes = contract.get("lanes") or {}
if not lanes:
    problems.append("contract declares no lanes")

for name, entry in sorted(lanes.items()):
    minimum = entry.get("min_os", "")
    platform = entry.get("platform", "")
    if not version.match(str(minimum)):
        problems.append(f"lane {name}.min_os is not MAJOR.MINOR: {minimum!r}")
        continue
    if platform not in platforms:
        problems.append(f"lane {name} names unknown platform {platform!r}")
        continue
    floor = platforms[platform]["deployment_target"]
    as_tuple = lambda value: tuple(int(part) for part in value.split("."))
    if as_tuple(minimum) < as_tuple(floor):
        problems.append(
            f"lane {name} claims min_os {minimum}, below the {platform} floor {floor}"
        )

if problems:
    for problem in problems:
        print(f"PROBLEM\t{problem}")
    sys.exit(1)

for name, entry in sorted(platforms.items()):
    print(f"PLATFORM\t{name}\t{entry['deployment_target']}\t{entry.get('swiftpm_platform', '')}")
for name, entry in sorted(lanes.items()):
    print(f"LANE\t{name}\t{entry['platform']}\t{entry['min_os']}")
PY
)"; then
    err "contract is unreadable or self-inconsistent:"
    printf '%s\n' "$contract_values" | sed 's/^PROBLEM\t/  - /' >&2
    exit 1
fi

ios_floor=""
macos_floor=""
ios_swiftpm=""
macos_swiftpm=""
perfplus_min=""

while IFS=$'\t' read -r kind name a b; do
    case "$kind:$name" in
        PLATFORM:ios)   ios_floor="$a";   ios_swiftpm="$b" ;;
        PLATFORM:macos) macos_floor="$a"; macos_swiftpm="$b" ;;
        LANE:ios_performance_plus) perfplus_min="$b" ;;
    esac
done <<<"$contract_values"

for required in ios_floor macos_floor ios_swiftpm macos_swiftpm perfplus_min; do
    if [ -z "${!required}" ]; then
        err "contract is missing $required"
        exit 1
    fi
done

info "contract: iOS $ios_floor ($ios_swiftpm), macOS $macos_floor ($macos_swiftpm)"
info "contract: Performance+ minimum iOS $perfplus_min"

# ---------------------------------------------------------------------------
# 2. Derived declarations
# ---------------------------------------------------------------------------

check_literal() {
    local file="$1" literal="$2" why="$3"
    if [ ! -f "$file" ]; then
        fail "missing derived consumer: ${file#$REPO_ROOT/}"
        return
    fi
    if ! grep -qF -- "$literal" "$file"; then
        fail "${file#$REPO_ROOT/} does not carry '$literal' ($why)"
        return
    fi
    info "ok: ${file#$REPO_ROOT/} carries '$literal'"
}

check_literal "$PACKAGE_SWIFT" "$ios_swiftpm"   "SwiftPM iOS platform must match the contract"
check_literal "$PACKAGE_SWIFT" "$macos_swiftpm" "SwiftPM macOS platform must match the contract"
check_literal "$CORE_PACKAGE_SWIFT" "$ios_swiftpm" \
    "engine-free SwiftPM iOS platform must match the contract"
check_literal "$CORE_PACKAGE_SWIFT" "$macos_swiftpm" \
    "engine-free SwiftPM macOS platform must match the contract"

swift_tuple() {
    printf '(major: %s, minor: %s)' "${1%%.*}" "${1##*.}"
}

check_literal "$FLOOR_SWIFT" "let iOS = $(swift_tuple "$ios_floor")" \
    "the runtime mirror of the iOS floor"
check_literal "$FLOOR_SWIFT" "let macOS = $(swift_tuple "$macos_floor")" \
    "the runtime mirror of the macOS floor"
check_literal "$FLOOR_SWIFT" "let performancePlusMinimumIOS = $(swift_tuple "$perfplus_min")" \
    "the runtime mirror of the Performance+ lane minimum"

# The build script is asked, not read.
#
# Grepping it for the literal would have been the easy check and the wrong one:
# it would pass for a script that contains the number in a comment and computes
# something else, and it would fail for the script that does the right thing --
# reads the contract at build time and holds no copy at all. What matters is
# the value the build would actually use, so the gate runs the script's own
# reporting mode and compares that. It needs no macOS: the mode exists precisely
# so this check runs everywhere.
if [ -f "$BUILD_SCRIPT" ]; then
    for platform in ios macos; do
        case "$platform" in
            ios)   expected="$ios_floor" ;;
            macos) expected="$macos_floor" ;;
        esac
        if ! reported="$(bash "$BUILD_SCRIPT" --print-deployment-target "$platform" 2>&1)"; then
            fail "build-apple-sdk.sh could not report its $platform deployment target: $reported"
            continue
        fi
        if [ "$reported" != "$expected" ]; then
            fail "build-apple-sdk.sh would build $platform against $reported, contract says $expected"
            continue
        fi
        info "ok: build-apple-sdk.sh reports $platform $reported"
    done
else
    info "skip: $BUILD_SCRIPT does not exist yet"
fi

# ---------------------------------------------------------------------------
# 3. Nobody else sets a deployment target
# ---------------------------------------------------------------------------
#
# Derived from the tree. Anything matching outside the allowlist is a second
# copy of the decision, which is the drift this gate is for.
#
# --untracked matters locally and is a no-op in CI, which checks out only
# tracked files. Without it the sweep reports clean on a working tree whose new
# consumer simply has not been committed yet -- silence read as an answer. A
# source archive has no Git metadata, so the same sweep falls back to ripgrep.
#
# The repository's ignore rules are part of source discovery. Ignored build
# products, generated output, and local tooling content are not valid source
# declarations for this sweep; a new declaration belongs in a non-ignored
# source path. The fallback asks ripgrep to use the repository's ignore rules
# even when a source copy has no .git directory.

allowed_to_declare() {
    case "$1" in
        contracts/apple/deployment-floor.json) return 0 ;;
        platforms/apple/Package.swift) return 0 ;;
        platforms/apple/core/Package.swift) return 0 ;;
        platforms/apple/core/Sources/MigoAppleCore/MigoDeploymentFloor.swift) return 0 ;;
        scripts/build-apple-sdk.sh) return 0 ;;
        scripts/test-apple-deployment-floor-contract.sh) return 0 ;;
        docs/*) return 0 ;;
        *) return 1 ;;
    esac
}

pattern='IPHONEOS_DEPLOYMENT_TARGET|MACOSX_DEPLOYMENT_TARGET|\.iOS\(\.v|\.macOS\(\.v'
sweep_hits=0
unexpected=0
scanner_failed=0

excluded_trees=(
    '.git'
    'out'
    'dist'
    'platforms/android/.gradle'
    'platforms/openharmony/.hvigor'
)
git_pathspecs=()
rg_globs=()
for excluded_tree in ${excluded_trees[@]+"${excluded_trees[@]}"}; do
    git_pathspecs+=(":(exclude,glob)$excluded_tree" ":(exclude,glob)$excluded_tree/**")
    rg_globs+=(--glob "!/$excluded_tree" --glob "!/$excluded_tree/**")
done

scan_deployment_target_declarations() (
    local output status stderr_file git_root_candidate git_root normalized_git_root
    local cat_status cleanup_status

    stderr_file=""
    cleanup_scanner_temp() {
        if [ -n "${stderr_file:-}" ]; then
            rm -f -- "$stderr_file"
        fi
    }
    abort_scanner() {
        cleanup_scanner_temp
        exit 125
    }
    disable_scanner_traps() {
        trap - EXIT HUP INT TERM
    }
    remove_scanner_temp() {
        if ! rm -f -- "$stderr_file"; then
            return 1
        fi
        stderr_file=""
        disable_scanner_traps
    }
    trap cleanup_scanner_temp EXIT
    trap abort_scanner HUP INT TERM

    if ! stderr_file="$(mktemp "${TMPDIR:-/tmp}/migo-apple-floor-scanner.XXXXXX")"; then
        err "deployment-target scanner could not create a temporary stderr file" >&2
        return 125
    fi
    if [ -z "$stderr_file" ]; then
        err "deployment-target scanner received an empty temporary stderr path" >&2
        return 125
    fi

    git_root=""
    if git_root_candidate="$(git -C "$REPO_ROOT" rev-parse --show-toplevel 2>/dev/null)" \
        && normalized_git_root="$(cd "$git_root_candidate" 2>/dev/null && pwd -P)" \
        && [ "$normalized_git_root" = "$REPO_ROOT" ]; then
        git_root="$normalized_git_root"
    fi

    if [ -n "$git_root" ]; then
        if output="$(git -C "$REPO_ROOT" grep --untracked -lE "$pattern" -- ${git_pathspecs[@]+"${git_pathspecs[@]}"} 2>"$stderr_file")"; then
            status=0
        else
            status=$?
        fi
    else
        if output="$(
            {
                cd "$REPO_ROOT" || exit 125
                rg --hidden --no-require-git --files-with-matches \
                    ${rg_globs[@]+"${rg_globs[@]}"} -e "$pattern" .
            } 2>"$stderr_file"
        )"; then
            status=0
        else
            status=$?
        fi
        output="${output//$'\n./'/$'\n'}"
        output="${output#./}"
    fi

    if [ -s "$stderr_file" ]; then
        cat -- "$stderr_file" >&2
        cat_status=$?
        if remove_scanner_temp; then
            cleanup_status=0
        else
            cleanup_status=$?
        fi
        if [ "$cat_status" -ne 0 ] || [ "$cleanup_status" -ne 0 ]; then
            return 125
        fi
        return 125
    fi
    if ! remove_scanner_temp; then
        err "deployment-target scanner could not clean up its temporary stderr file" >&2
        return 125
    fi

    # Both scanners use 1 for a successful search with no matches. Let the
    # zero-hit guard below issue the more precise fail-closed diagnostic.
    if [ "$status" -eq 1 ]; then
        return 0
    fi
    if [ "$status" -ne 0 ]; then
        return "$status"
    fi
    printf '%s\n' "$output"
)

if ! sweep_output="$(scan_deployment_target_declarations)"; then
    fail "deployment-target declaration scan failed"
    scanner_failed=1
else
    while IFS= read -r relative; do
        [ -n "$relative" ] || continue
        sweep_hits=$((sweep_hits + 1))
        if ! allowed_to_declare "$relative"; then
            fail "$relative declares an Apple deployment target; derive it from contracts/apple/deployment-floor.json"
            unexpected=$((unexpected + 1))
        fi
    done <<<"$sweep_output"
fi

if [ "$scanner_failed" -ne 0 ]; then
    : # The scanner-specific failure above is the authoritative diagnostic.
elif [ "$sweep_hits" -eq 0 ]; then
    fail "the deployment-target sweep matched nothing; a sweep that finds no declarations is broken, not clean"
else
    info "sweep: $sweep_hits file(s) name a deployment target, $unexpected outside the derived set"
fi

# ---------------------------------------------------------------------------
# 5. The artifact, not the declaration
# ---------------------------------------------------------------------------

if [ -n "$ARTIFACT_DIR" ]; then
    if [ ! -d "$ARTIFACT_DIR" ]; then
        err "--artifacts $ARTIFACT_DIR is not a directory"
        exit 1
    fi
    reader=""
    if command -v vtool >/dev/null 2>&1; then
        reader="vtool"
    elif command -v otool >/dev/null 2>&1; then
        reader="otool"
    else
        err "--artifacts needs vtool or otool; neither is on PATH."
        err "Refusing to report a pass without reading the binaries."
        exit 1
    fi

    checked=0
    while IFS= read -r macho; do
        case "$macho" in *.a|*.dylib|*.o) ;; *) continue ;; esac
        checked=$((checked + 1))
        if [ "$reader" = "vtool" ]; then
            load_output="$(vtool -show-build "$macho" 2>/dev/null)"
        else
            load_output="$(otool -l "$macho" 2>/dev/null)"
        fi
        minos="$(printf '%s\n' "$load_output" | awk '/minos/ { print $2; exit }')"
        platform="$(printf '%s\n' "$load_output" | awk '/platform/ { print $2; exit }')"
        if [ -z "$minos" ]; then
            fail "no LC_BUILD_VERSION in ${macho#$ARTIFACT_DIR/}"
            continue
        fi
        case "$platform" in
            *IOS*|*ios*|1|7) expected="$ios_floor" ;;
            *MACOS*|*macos*|6) expected="$macos_floor" ;;
            *) fail "unrecognised platform '$platform' in ${macho#$ARTIFACT_DIR/}"; continue ;;
        esac
        if [ "$minos" != "$expected" ]; then
            fail "${macho#$ARTIFACT_DIR/} was built with minos $minos, contract says $expected"
        fi
    done < <(find "$ARTIFACT_DIR" -type f 2>/dev/null)

    if [ "$checked" -eq 0 ]; then
        fail "--artifacts $ARTIFACT_DIR contained no Mach-O files to read"
    else
        info "artifacts: read LC_BUILD_VERSION from $checked file(s) using $reader"
    fi
else
    info "skip: artifact check needs --artifacts DIR on a macOS runner"
fi

# ---------------------------------------------------------------------------

if [ "$failures" -ne 0 ]; then
    err "Apple deployment floor contract: FAIL ($failures problem(s))"
    exit 1
fi

ok "Apple deployment floor contract: PASS"
exit 0
