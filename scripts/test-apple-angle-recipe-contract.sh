#!/usr/bin/env bash
# The Apple ANGLE recipe must be derivable, total, and fail closed.
#
# THE DRIFT THIS EXISTS TO CATCH is not hypothetical, it is what building ANGLE
# for Apple cost to learn in the first place. .github/workflows/apple-angle-probe.yml
# (PR #185) spent a 12 GB checkout establishing three facts a blind script would
# have got wrong, and two of them are exactly the kind that stay wrong quietly:
#
#   - `target_os="ios"` is not a configuration. gn's mobile_config.gni asserts
#     target_environment is one of simulator/device/catalyst. A build script that
#     omitted it would fail loudly -- but one that set the WRONG one would
#     succeed and produce a simulator library named as a device library, and the
#     first symptom would be an app that will not launch on a phone.
#   - The slices ANGLE is built for have to be the slices the engine is built
#     for. Those two xcframeworks are linked into the same application; if they
#     disagree about which architectures exist, the failure lands in a consumer's
#     link step naming neither script. scripts/build-apple-sdk.sh owns that list,
#     so scripts/build-angle-apple.sh asks it (--print-slices) rather than
#     keeping a copy, and this gate checks the mapping is TOTAL over what that
#     answer contains.
#
# It also keeps the two numbers that already have an owner from acquiring a
# second one: the deployment targets live in contracts/apple/deployment-floor.json
# and the revision and gn arguments live in
# contracts/artifact-manifest/apple-angle.lock.json. A copy in the build script
# is the one that silently wins on the build machine.
#
# Host-only: it asks the scripts questions and reads their answers. Nothing here
# needs macOS, which is the point -- the expensive lane runs on a cadence, and
# the properties that can be checked on every pull request should be.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

pass() { printf '\033[0;32m[ok]\033[0m %s\n' "$*"; }
bad()  { printf '\033[0;31m[FAIL]\033[0m %s\n' "$*" >&2; }

# A triple that is not, and will not become, an Apple slice this project builds:
# the fail-closed control. If --print-gn-args ever answers for it, the totality
# check above became vacuous -- every slice would "have" a configuration.
UNMAPPED_CONTROL="aarch64-apple-tvos"

# ---------------------------------------------------------------------------
# The audit. One implementation, run against the real tree and against every
# injected fixture, so a violation this gate claims to catch is a violation it
# has been SEEN to catch.
#
# Every finding prints `VIOLATION <id> ...`, and the injections below assert on
# the id. Asserting only "it went red" is how a gate passes its own proof for
# the wrong reason -- an injection that breaks the fixture in some unrelated way
# turns it red just as well.
# ---------------------------------------------------------------------------
run_audit() {
    audit_root="$1"
    angle="$audit_root/scripts/build-angle-apple.sh"
    sdk="$audit_root/scripts/build-apple-sdk.sh"
    lock="$audit_root/contracts/artifact-manifest/apple-angle.lock.json"
    findings=0

    report() {
        printf 'VIOLATION %s: %s\n' "$1" "$2"
        findings=$((findings + 1))
    }

    for f in "$angle" "$sdk" "$lock"; do
        [ -f "$f" ] || { report missing-input "$f does not exist"; echo "$findings"; return 1; }
    done

    # --- the pin is a pin ----------------------------------------------------
    revision="$(python3 -c '
import json, sys
print(json.load(open(sys.argv[1]))["source"]["angle_revision"])' "$lock" 2>/dev/null || true)"
    if ! printf '%s' "$revision" | grep -Eq '^[0-9a-f]{40}$'; then
        report lock-revision-malformed \
            "source.angle_revision is not a 40-character commit hash: '${revision}'"
    fi

    common="$(python3 -c '
import json, sys
print(json.load(open(sys.argv[1]))["source"]["gn_args_common"])' "$lock" 2>/dev/null || true)"

    # The floor contract owns deployment targets. The lock owning one too would
    # be two answers to one question, and gn takes the last `--args` wins.
    if printf '%s' "$common" | grep -q 'deployment_target'; then
        report lock-carries-deployment-target \
            "source.gn_args_common sets a deployment target; that number belongs to contracts/apple/deployment-floor.json"
    fi

    # --- the recipe carries no second copy of the pin ------------------------
    # Comment lines stripped first: prose may quote a hash, and a header that
    # explains where the pin lives is the opposite of the defect.
    if sed 's/#.*//' "$angle" | grep -Eq '[0-9a-f]{40}'; then
        report revision-hardcoded \
            "scripts/build-angle-apple.sh contains a 40-character hash outside a comment; the revision belongs to the lock file"
    fi

    # --- fail closed ---------------------------------------------------------
    if bash "$angle" --print-gn-args "$UNMAPPED_CONTROL" >/dev/null 2>&1; then
        report not-fail-closed \
            "--print-gn-args answered for $UNMAPPED_CONTROL, so the totality check below proves nothing"
    fi

    # --- total over the engine's own slices ----------------------------------
    for platform in ios ios-simulator macos; do
        if ! slices="$(bash "$sdk" --print-slices "$platform" 2>/dev/null)"; then
            report slices-unavailable "build-apple-sdk.sh --print-slices $platform failed"
            continue
        fi
        case "$platform" in
            macos) floor=macos ;;
            *)     floor=ios ;;
        esac
        if ! want_target="$(bash "$sdk" --print-deployment-target "$floor" 2>/dev/null)"; then
            report floor-unavailable "build-apple-sdk.sh --print-deployment-target $floor failed"
            continue
        fi

        for triple in $slices; do
            if ! args="$(bash "$angle" --print-gn-args "$triple" 2>/dev/null)"; then
                report unmapped-slice \
                    "$platform slice $triple has no ANGLE configuration"
                continue
            fi

            case "$args" in
                *"$common"*) ;;
                *) report gn-args-not-from-lock \
                       "$triple: the arguments do not contain the lock's gn_args_common verbatim" ;;
            esac

            case "$triple" in
                aarch64-*) want_cpu=arm64 ;;
                x86_64-*)  want_cpu=x64 ;;
                *)         want_cpu="" ;;
            esac
            if [ -n "$want_cpu" ]; then
                case "$args" in
                    *"target_cpu=\"$want_cpu\""*) ;;
                    *) report target-cpu-mismatch \
                           "$triple: expected target_cpu=\"$want_cpu\"" ;;
                esac
            fi

            case "$triple" in
                *-apple-ios|*-apple-ios-sim)
                    case "$args" in
                        *'target_os="ios"'*) ;;
                        *) report os-mismatch "$triple: expected target_os=\"ios\"" ;;
                    esac
                    # device for the one device triple, simulator for both
                    # simulator triples. x86_64-apple-ios is a SIMULATOR target:
                    # there has never been an Intel iOS device.
                    case "$triple" in
                        aarch64-apple-ios) want_env=device ;;
                        *)                 want_env=simulator ;;
                    esac
                    case "$args" in
                        *'target_environment='*)
                            case "$args" in
                                *"target_environment=\"$want_env\""*) ;;
                                *) report ios-wrong-target-environment \
                                       "$triple: expected target_environment=\"$want_env\"" ;;
                            esac
                            ;;
                        *)
                            report ios-missing-target-environment \
                                "$triple: target_os=\"ios\" without target_environment; gn asserts on this"
                            ;;
                    esac
                    case "$args" in
                        *"ios_deployment_target=\"$want_target\""*) ;;
                        *) report deployment-target-drift \
                               "$triple: expected ios_deployment_target=\"$want_target\" from the floor contract" ;;
                    esac
                    ;;
                *-apple-darwin)
                    case "$args" in
                        *'target_os="mac"'*) ;;
                        *) report os-mismatch "$triple: expected target_os=\"mac\"" ;;
                    esac
                    case "$args" in
                        *'target_environment='*)
                            report mac-has-target-environment \
                                "$triple: target_environment is an iOS-only argument" ;;
                        *) ;;
                    esac
                    case "$args" in
                        *"mac_deployment_target=\"$want_target\""*) ;;
                        *) report deployment-target-drift \
                               "$triple: expected mac_deployment_target=\"$want_target\" from the floor contract" ;;
                    esac
                    ;;
                *)
                    report unknown-slice-shape \
                        "$triple: build-apple-sdk.sh reports a slice this gate does not know how to check"
                    ;;
            esac
        done
    done

    echo "$findings"
    [ "$findings" -eq 0 ]
}

# ---------------------------------------------------------------------------
# The real tree
# ---------------------------------------------------------------------------
failures=0
output="$(run_audit "$ROOT" 2>&1)" && status=0 || status=$?
if [ "$status" -eq 0 ]; then
    pass "the Apple ANGLE recipe is total over the engine's slices and derives every pinned value"
else
    bad "the recipe violates the contract:"
    printf '%s\n' "$output" | sed 's/^/    /' >&2
    failures=$((failures + 1))
fi

# ---------------------------------------------------------------------------
# Injections. Each one must turn the audit red FOR ITS OWN REASON.
# ---------------------------------------------------------------------------
WORK="$(mktemp -d "${TMPDIR:-/tmp}/migo-angle-recipe.XXXXXX")"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

fixture() {
    dest="$WORK/$1"
    rm -rf "$dest"
    mkdir -p "$dest/scripts" "$dest/contracts/artifact-manifest" "$dest/contracts/apple"
    cp "$ROOT/scripts/build-angle-apple.sh" "$dest/scripts/"
    cp "$ROOT/scripts/build-apple-sdk.sh" "$dest/scripts/"
    cp "$ROOT/contracts/artifact-manifest/apple-angle.lock.json" "$dest/contracts/artifact-manifest/"
    cp "$ROOT/contracts/apple/deployment-floor.json" "$dest/contracts/apple/"
    printf '%s' "$dest"
}

expect_violation() {
    what="$1"; want_id="$2"; dest="$3"
    out="$(run_audit "$dest" 2>&1)" && rc=0 || rc=$?
    if [ "$rc" -eq 0 ]; then
        bad "injection '$what' did not turn the audit red"
        failures=$((failures + 1))
        return
    fi
    if printf '%s\n' "$out" | grep -q "^VIOLATION $want_id:"; then
        pass "injection '$what' -> $want_id"
    else
        bad "injection '$what' went red, but not as $want_id. What it reported:"
        printf '%s\n' "$out" | sed 's/^/    /' >&2
        failures=$((failures + 1))
    fi
}

# The clean fixture must be green, or every injection below proves nothing: a
# copy that is already red goes red for any reason at all.
dest="$(fixture control)"
if out="$(run_audit "$dest" 2>&1)"; then
    pass "the unmodified fixture is clean, so each injection below is the only difference"
else
    bad "the unmodified fixture is already red; no injection below proves anything:"
    printf '%s\n' "$out" | sed 's/^/    /' >&2
    failures=$((failures + 1))
fi

# 1. A slice loses its configuration.
dest="$(fixture unmapped)"
python3 - "$dest/scripts/build-angle-apple.sh" <<'PY'
import pathlib, sys
p = pathlib.Path(sys.argv[1]); s = p.read_text()
start = s.index("        aarch64-apple-ios-sim)\n")
end = s.index("        x86_64-apple-ios)\n")
p.write_text(s[:start] + s[end:])
PY
expect_violation "a slice arm is deleted" unmapped-slice "$dest"

# 2. The mapping stops failing closed. Without this control, check 1 would pass
#    for a script that answered anything for everything.
dest="$(fixture open)"
python3 - "$dest/scripts/build-angle-apple.sh" <<'PY'
import pathlib, sys
p = pathlib.Path(sys.argv[1]); s = p.read_text()
s = s.replace('''        *)
            err "no ANGLE configuration for Rust target: $triple"''',
'''        *)
            printf '%s target_os="mac" target_cpu="arm64"' "$GN_ARGS_COMMON"; return 0 ;;
        __never__)
            err "no ANGLE configuration for Rust target: $triple"''', 1)
p.write_text(s)
PY
expect_violation "an unknown triple gets a default configuration" not-fail-closed "$dest"

# 3. The deployment target acquires a second owner.
dest="$(fixture floordrift)"
sed -i 's/"\$ios_target"/"17.0"/' "$dest/scripts/build-angle-apple.sh"
expect_violation "the iOS deployment target is written into the recipe" deployment-target-drift "$dest"

# 4. target_environment goes missing on an iOS configuration.
dest="$(fixture noenv)"
sed -i 's/ target_environment="device"//' "$dest/scripts/build-angle-apple.sh"
expect_violation "an iOS configuration drops target_environment" ios-missing-target-environment "$dest"

# 5. ...or is present and wrong, which is the one that would build silently.
dest="$(fixture wrongenv)"
sed -i 's/target_environment="device"/target_environment="simulator"/' "$dest/scripts/build-angle-apple.sh"
expect_violation "the device configuration is built for the simulator" ios-wrong-target-environment "$dest"

# 6. A macOS configuration picks up the iOS-only argument.
dest="$(fixture macenv)"
python3 - "$dest/scripts/build-angle-apple.sh" <<'PY'
import pathlib, sys
p = pathlib.Path(sys.argv[1]); s = p.read_text()
s = s.replace('%s target_os="mac" target_cpu="arm64" mac_deployment_target="%s"',
              '%s target_os="mac" target_environment="device" target_cpu="arm64" mac_deployment_target="%s"', 1)
p.write_text(s)
PY
expect_violation "a macOS configuration sets target_environment" mac-has-target-environment "$dest"

# 7. An architecture is built for the wrong CPU.
dest="$(fixture cpu)"
python3 - "$dest/scripts/build-angle-apple.sh" <<'PY'
import pathlib, sys
p = pathlib.Path(sys.argv[1]); s = p.read_text()
s = s.replace('%s target_os="mac" target_cpu="x64" mac_deployment_target="%s"',
              '%s target_os="mac" target_cpu="arm64" mac_deployment_target="%s"', 1)
p.write_text(s)
PY
expect_violation "the Intel macOS slice is configured as arm64" target-cpu-mismatch "$dest"

# 8. The revision gets a second copy in the build script.
dest="$(fixture pin)"
python3 - "$dest/scripts/build-angle-apple.sh" <<'PY'
import pathlib, sys
p = pathlib.Path(sys.argv[1]); s = p.read_text()
s = s.replace('MODE=""\n',
              'MODE=""\nFALLBACK_REVISION="52f594287836c9970b67920da0633077aee42649"\n', 1)
p.write_text(s)
PY
expect_violation "the recipe carries its own copy of the revision" revision-hardcoded "$dest"

# 9. The gn arguments stop coming from the lock.
dest="$(fixture gnargs)"
sed -i 's/"\$GN_ARGS_COMMON"/"is_debug=false angle_enable_metal=true"/g' "$dest/scripts/build-angle-apple.sh"
expect_violation "the gn arguments are written into the recipe" gn-args-not-from-lock "$dest"

# 10. The lock file's pin stops being a pin.
dest="$(fixture badpin)"
sed -i 's/"angle_revision": "[0-9a-f]*"/"angle_revision": "main"/' "$dest/contracts/artifact-manifest/apple-angle.lock.json"
expect_violation "the pin is a branch name" lock-revision-malformed "$dest"

# 11. The lock file starts answering the floor contract's question.
dest="$(fixture lockfloor)"
python3 - "$dest/contracts/artifact-manifest/apple-angle.lock.json" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1]); doc = json.loads(p.read_text())
doc["source"]["gn_args_common"] += ' ios_deployment_target="17.0"'
p.write_text(json.dumps(doc, indent=2))
PY
expect_violation "the lock file carries a deployment target" lock-carries-deployment-target "$dest"

if [ "$failures" -ne 0 ]; then
    bad "$failures check(s) failed"
    exit 1
fi
echo "PASS: the Apple ANGLE recipe contract holds, and 11 injections were each seen to break it"
