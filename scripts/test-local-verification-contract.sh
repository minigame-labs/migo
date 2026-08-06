#!/usr/bin/env bash
# Contract for scripts/verify-change.sh.
#
# The properties here are the ones whose absence would make the script worse than
# nothing -- a verification entry point that reports a pass it did not earn is how
# three Android compile errors survived several sessions of green host runs.
#
#   * a crate source the module walk cannot reach stops the run, because an
#     unreached file has unknown platform conditions;
#   * a required target with no local build is reported NOT PROVEN and fails, never
#     skipped;
#   * the scope is what the flags say it is, so a narrow run cannot be read as a
#     broad one.
#
# The fixtures are throwaway repositories holding the real script and the real
# selector, so the contract is against the shipped code rather than a paraphrase.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

failures=0

fail() {
    printf '\033[0;31m[FAIL] %s\033[0m\n' "$*" >&2
    failures=$((failures + 1))
}

pass() {
    printf '\033[0;32m[ok]\033[0m %s\n' "$*"
}

assert_contains() {
    local haystack="$1" needle="$2" what="$3"
    if grep -qF -- "$needle" <<< "$haystack"; then
        pass "$what"
    else
        fail "$what -- expected to find '$needle' in:"
        printf '%s\n' "$haystack" >&2
    fi
}

assert_absent() {
    local haystack="$1" needle="$2" what="$3"
    if grep -qF -- "$needle" <<< "$haystack"; then
        fail "$what -- did not expect '$needle' in:"
        printf '%s\n' "$haystack" >&2
    else
        pass "$what"
    fi
}

assert_status() {
    local actual="$1" expected="$2" what="$3"
    if [[ "$actual" == "$expected" ]]; then
        pass "$what"
    else
        fail "$what -- expected exit $expected, got $actual"
    fi
}

assert_no_line_starting() {
    local haystack="$1" prefix="$2" what="$3"
    if grep -q "^$prefix" <<< "$haystack"; then
        fail "$what -- found a line starting '$prefix' in:"
        printf '%s\n' "$haystack" >&2
    else
        pass "$what"
    fi
}

git_quiet() {
    git -c user.name=fixture -c user.email=fixture@example.invalid -C "$1" "${@:2}"
}

# Builds a repository holding the real script, the real selector, and an engine
# workspace whose host suites pass.
#
# The workspace is what makes the verdict assertions mean anything. With no engine
# to build, every host step fails, the run exits non-zero for that reason, and an
# assertion on the exit code alone cannot tell "failed because a target was not
# proven" from "failed because the fixture has no engine" -- it would pass with the
# NOT PROVEN rule deleted. So the stubs are real crates that really compile.
STUB_CRATES=(
    demo
    shared io runtime-v8 audio graphics core capi platform
)

# Mirrors the real tree's third category: `engine/testing/` holds crates that
# measure the engine and ship nowhere. Placed faithfully rather than folded into
# `crates/` so the fixture also exercises the audit's indifference to it.
STUB_TESTING_CRATES=(
    alloc-probe
    contention-probe
    executor-probe
)

add_stub_crate() {
    local repo="$1" group="$2" name="$3"
    mkdir -p "$repo/engine/$group/$name/src"
    cat > "$repo/engine/$group/$name/Cargo.toml" <<EOF
[package]
name = "migo-$name"
version = "0.0.0"
edition = "2021"
EOF
    cat > "$repo/engine/$group/$name/src/lib.rs" <<'EOF'
pub fn present() -> bool {
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn present() {
        assert!(super::present());
    }
}
EOF
}

new_fixture() {
    local name="$1"
    local repo="$WORK/$name"
    mkdir -p "$repo/scripts/lib" "$repo/docs" "$repo/engine"
    cp "$ROOT/scripts/verify-change.sh" "$repo/scripts/"
    cp "$ROOT/scripts/lib/verification_targets.py" "$repo/scripts/lib/"
    {
        printf '[workspace]\nresolver = "2"\nmembers = [\n'
        for crate in "${STUB_CRATES[@]}"; do
            printf ' "crates/%s",\n' "$crate"
        done
        for crate in "${STUB_TESTING_CRATES[@]}"; do
            printf ' "testing/%s",\n' "$crate"
        done
        printf ']\n'
    } > "$repo/engine/Cargo.toml"
    for crate in "${STUB_CRATES[@]}"; do
        add_stub_crate "$repo" crates "$crate"
    done
    for crate in "${STUB_TESTING_CRATES[@]}"; do
        add_stub_crate "$repo" testing "$crate"
    done
    printf 'baseline\n' > "$repo/docs/note.md"
    git_quiet "$repo" init -q -b master
    git_quiet "$repo" add -A
    git_quiet "$repo" commit -q -m "baseline"
    echo "$repo"
}

run_verify() {
    local repo="$1"
    shift
    ( cd "$repo" && bash scripts/verify-change.sh "$@" 2>&1 )
}

# ------------------------------------------------------------
# An unreached crate source stops the run.
# ------------------------------------------------------------
repo="$(new_fixture unreachable)"
printf 'pub fn orphaned() {}\n' > "$repo/engine/crates/demo/src/orphan.rs"
status=0
output="$(run_verify "$repo" --plan-only)" || status=$?
assert_status "$status" 1 "an unreached crate source fails the run"
assert_contains "$output" "engine/crates/demo/src/orphan.rs" \
    "the unreached file is named"

# The tree-wide half of it: an unreached file nobody touched still stops the run.
# Committed on the base, so it is outside the changed set and only the audit can
# see it -- which is the point of auditing the whole tree rather than the diff.
repo="$(new_fixture unreached_untouched)"
printf 'pub fn orphaned() {}\n' > "$repo/engine/crates/demo/src/orphan.rs"
git_quiet "$repo" add -A
git_quiet "$repo" commit -q -m "orphan"
printf 'edited\n' > "$repo/docs/note.md"
status=0
output="$(run_verify "$repo" --plan-only)" || status=$?
assert_status "$status" 1 "an unreached file outside the changed set fails the run"
assert_contains "$output" "engine/crates/demo/src/orphan.rs" \
    "the untouched unreached file is named"

# The other half: a crate directory with no manifest is invisible to the tree
# audit, so the changed file's unknown conditions have to stop the run on their own.
repo="$(new_fixture unmanifested)"
mkdir -p "$repo/engine/crates/nomanifest/src"
printf 'pub mod known;\n' > "$repo/engine/crates/nomanifest/src/lib.rs"
printf 'pub fn known() {}\n' > "$repo/engine/crates/nomanifest/src/known.rs"
printf 'pub fn orphaned() {}\n' > "$repo/engine/crates/nomanifest/src/orphan.rs"
status=0
output="$(run_verify "$repo" --plan-only)" || status=$?
assert_status "$status" 1 "a changed source with unknown conditions fails the run"
assert_contains "$output" "UNDETERMINED" "the unknown-condition file is reported"

# ------------------------------------------------------------
# A change outside the engine needs no target build.
# ------------------------------------------------------------
repo="$(new_fixture documentation)"
printf 'edited\n' > "$repo/docs/note.md"
status=0
output="$(run_verify "$repo" --plan-only)" || status=$?
assert_status "$status" 0 "a documentation change plans cleanly"
assert_absent "$output" "TARGET" "a documentation change requires no target"

# ------------------------------------------------------------
# A target with no local build is NOT PROVEN, and that fails.
# ------------------------------------------------------------
repo="$(new_fixture ohos)"
cat > "$repo/engine/crates/demo/src/lib.rs" <<'RUST'
#[cfg(all(target_os = "linux", target_env = "ohos"))]
pub mod ohos;
RUST
printf 'pub fn present() {}\n' > "$repo/engine/crates/demo/src/ohos.rs"
status=0
output="$(run_verify "$repo" --plan-only)" || status=$?
assert_status "$status" 0 "an OpenHarmony-conditional change plans cleanly"
assert_contains "$output" "TARGET ohos compile" \
    "OpenHarmony is recognised through target_env, not target_os"

status=0
output="$(run_verify "$repo")" || status=$?
assert_status "$status" 1 "a target with no local build fails the run"
assert_contains "$output" "NOT PROVEN" "the missing target build is reported, not skipped"
assert_contains "$output" "ohos compile" "the unproven target is named"
# The discriminating part. Without it, the exit code above is also produced by a
# fixture whose host steps fail, so the assertion would hold with the NOT PROVEN
# rule deleted.
assert_no_line_starting "$output" "FAIL" \
    "NOT PROVEN is the only thing that failed this run"

# ------------------------------------------------------------
# A run with nothing to prove passes. Without this, an always-red script would
# satisfy every assertion above.
# ------------------------------------------------------------
repo="$(new_fixture clean)"
printf 'edited\n' > "$repo/docs/note.md"
status=0
output="$(run_verify "$repo")" || status=$?
assert_status "$status" 0 "a change needing no target build passes"
assert_contains "$output" "verified for every target this change touches" \
    "the passing verdict is stated"
assert_absent "$output" "NOT PROVEN" "nothing is left unproven"

# ------------------------------------------------------------
# Android's two tiers.
# ------------------------------------------------------------
repo="$(new_fixture android)"
cat > "$repo/engine/crates/demo/src/lib.rs" <<'RUST'
#[cfg(target_os = "android")]
pub mod android;
RUST
printf 'pub fn attach() {}\n' > "$repo/engine/crates/demo/src/android.rs"
status=0
output="$(run_verify "$repo" --plan-only)" || status=$?
assert_status "$status" 0 "an Android-conditional change plans cleanly"
assert_contains "$output" "TARGET android compile" "the Android compile tier is selected"

repo="$(new_fixture cdylib)"
mkdir -p "$repo/engine/crates/android-jni/src"
printf '[package]\nname = "migo-android-jni"\n' > "$repo/engine/crates/android-jni/Cargo.toml"
printf 'pub fn register() {}\n' > "$repo/engine/crates/android-jni/src/lib.rs"
status=0
output="$(run_verify "$repo" --plan-only)" || status=$?
assert_status "$status" 0 "a cdylib change plans cleanly"
assert_contains "$output" "TARGET android link" \
    "the cdylib asks for a link, which compiling its dependencies does not prove"

# ------------------------------------------------------------
# Scope is what the flags say.
# ------------------------------------------------------------
repo="$(new_fixture scope)"
git_quiet "$repo" checkout -q -b feature
cat > "$repo/engine/crates/demo/src/lib.rs" <<'RUST'
#[cfg(target_os = "android")]
pub mod android;
RUST
printf 'pub fn attach() {}\n' > "$repo/engine/crates/demo/src/android.rs"
git_quiet "$repo" add -A
git_quiet "$repo" commit -q -m "android module"
status=0
output="$(run_verify "$repo" --plan-only --base HEAD)" || status=$?
assert_status "$status" 0 "a working-tree scope plans cleanly"
assert_absent "$output" "TARGET" \
    "--base HEAD covers the working tree only, so a committed change is out of scope"
assert_contains "$output" "HEAD..HEAD plus the working tree" "the scope is stated"

status=0
output="$(run_verify "$repo" --plan-only --base master)" || status=$?
assert_contains "$output" "TARGET android compile" \
    "the default branch scope covers the branch's own commits"

# ------------------------------------------------------------
# The Android compile tier covers the crates it claims to cover.
#
# Against the real workspace, not a fixture: --compile-only builds one package and
# relies on its dependency closure to reach the other three. Narrow that selection
# and the mode still prints SUCCESS while covering less, which is the failure this
# whole script exists to prevent -- so the closure is asserted rather than assumed.
# ------------------------------------------------------------
selection="$(grep -o 'package_args=(-p [a-z0-9-]*)' "$ROOT/scripts/build-android-so.sh" || true)"
if [[ -z "$selection" ]]; then
    fail "cannot find the package --compile-only selects in scripts/build-android-so.sh"
else
    selection="${selection#package_args=(-p }"
    selection="${selection%)}"
    closure="$(cd "$ROOT/engine" && cargo tree --offline -p "$selection" \
        --edges normal --prefix none --no-dedupe 2>/dev/null | awk '{print $1}' | sort -u)"
    for crate in migo-core migo-graphics migo-platform migo-capi; do
        assert_contains "$closure" "$crate" \
            "--compile-only reaches $crate through $selection"
    done
fi

# ------------------------------------------------------------
# CI runs every host suite this entry point runs.
#
# The gap this closes is the one that made the entry point necessary: two lists of
# crates in two files, one of them missing four crates, and nothing comparing them.
# ------------------------------------------------------------
#
# The local half asks the script rather than grepping it. It used to grep for
# `cargo test -p migo-...`, which coupled this check to a spelling: the day the
# host steps stopped carrying the word `cargo` themselves, the grep matched
# nothing -- and with no `|| true` under `set -euo pipefail`, this file died at
# that assignment, *before* reaching the `fail` below that exists to report
# exactly that case. It failed with no message at all, which is the vacuous
# failure the block underneath already warned about in its own comment and did
# not apply here. Both extractions are now non-fatal so the empty case is
# reportable, and the local one has one authority instead of a regular
# expression guessing at one.
local_crates="$(bash "$ROOT/scripts/verify-change.sh" --list-host-crates 2>/dev/null || true)"
ci_crates="$(grep -E 'cargo test' "$ROOT/.github/workflows/pr-ci.yml" \
    | grep -oE '\-p migo-[a-z0-9-]+' | grep -oE 'migo-[a-z0-9-]+' | sort -u || true)"
if [[ -z "$ci_crates" ]]; then
    fail "cannot find the crates pr-ci.yml tests"
elif [[ -z "$local_crates" ]]; then
    fail "scripts/verify-change.sh --list-host-crates reported nothing"
else
    while read -r crate; do
        assert_contains "$ci_crates" "$crate" "pr-ci.yml runs $crate's tests too"
    done <<< "$local_crates"
fi

# ------------------------------------------------------------
# CI lints every crate it tests.
#
# The same two-lists-in-one-file drift, one step later. Clippy for graphics, core,
# capi, platform and audio ran in no job at all while their tests ran in one --
# nothing compared the lint list to the test list, so the omission was invisible.
# Comparing them means a crate added to a test line without a clippy line fails
# here rather than being linted nowhere for months.
#
# Split across two jobs on purpose (one installs system packages, one does not),
# so the check is against the file, not against any single job.
# ------------------------------------------------------------
#
# `|| true` because the whole point of the empty case is to report it: under
# `set -euo pipefail` a grep that matches nothing kills the script, and a contract
# that dies silently when its subject disappears is exactly the vacuous pass this
# file exists to prevent.
clippy_crates="$(grep -E 'cargo clippy' "$ROOT/.github/workflows/pr-ci.yml" \
    | grep -oE '\-p migo-[a-z0-9-]+' | grep -oE 'migo-[a-z0-9-]+' | sort -u || true)"
if [[ -z "$clippy_crates" ]]; then
    fail "cannot find the crates pr-ci.yml runs clippy on"
else
    while read -r crate; do
        assert_contains "$clippy_crates" "$crate" "pr-ci.yml lints $crate too"
    done <<< "$ci_crates"
fi

if [[ "$failures" -ne 0 ]]; then
    printf '\033[0;31m%s contract check(s) failed\033[0m\n' "$failures" >&2
    exit 1
fi
printf '\033[0;32mAll verify-change contract checks passed\033[0m\n'
