#!/usr/bin/env bash
# Run one package's filtered lib tests, and refuse a filter that matches nothing.
#
# A filtered `cargo test` exits 0 when the filter matches zero tests. That exit
# status is the evidence a red/green step is read from, so a typo in the filter
# is indistinguishable from a passing suite -- and it fails in the direction
# that hides work: the step reports green having run nothing. This repository
# has made that mistake more than once, which is why the filter is listed before
# it is run and a zero match is a hard failure.
#
# Usage:
#   scripts/run-cargo-lib-test-filter.sh <package> <filter>
#   scripts/run-cargo-lib-test-filter.sh --self-test
#
# The cargo invocation goes through `scripts/dev-test-host.sh`, which passes any
# subcommand through after establishing the system clang, the Khronos headers
# and the linux-gnu V8 archive. Several workspace members link Skia and do not
# build without it, so a bare `cargo test` here would fail for a reason that has
# nothing to do with the filter.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

err() { echo -e "\033[0;31m[test-filter] $*\033[0m" >&2; }
info() { echo -e "\033[0;36m[test-filter] $*\033[0m"; }
ok() { echo -e "\033[0;32m[test-filter] $*\033[0m"; }

# `cargo test -- --list` prints one `<path>: test` line per test and then a
# summary line. Reading stdin rather than running cargo is what lets the
# self-test below exercise this against known input.
count_listed_tests() {
    grep -cE '^[A-Za-z_][A-Za-z0-9_:]*: test$' || true
}

self_test() {
    local failures=0
    check() {
        local name="$1" expected="$2" input="$3" actual
        actual="$(printf '%s' "$input" | count_listed_tests)"
        if [[ "$actual" != "$expected" ]]; then
            err "self-test: $name counted $actual, expected $expected"
            failures=$((failures + 1))
        fi
    }

    check "zero matches" 0 ''
    check "zero matches with only a summary" 0 '0 tests, 0 benchmarks'
    check "one match" 1 'callback_id::tests::starts_at_one: test
1 test, 0 benchmarks'
    check "several matches" 3 'a::b::one: test
a::b::two: test
c::three: test
3 tests, 0 benchmarks'
    # A benchmark is not a test, and neither is prose that happens to contain
    # the word: counting either would make a zero-test filter look non-empty.
    check "benchmarks and prose are not tests" 1 'a::b::only: test
a::b::bench: benchmark
running 0 tests'

    if (( failures > 0 )); then
        err "self-test failed with $failures problem(s)"
        return 1
    fi
    ok "self-test passed: only the zero-match case fails"
}

if [[ "${1:-}" == "--self-test" ]]; then
    self_test
    exit $?
fi

if [[ $# -ne 2 ]]; then
    err "usage: $0 <package> <filter>   |   $0 --self-test"
    exit 2
fi

PACKAGE="$1"
FILTER="$2"

info "listing tests matching '$FILTER' in $PACKAGE"
listing="$(
    bash "$SCRIPT_DIR/dev-test-host.sh" test -p "$PACKAGE" "$FILTER" \
        --lib --locked --offline -- --list
)"
matched="$(printf '%s\n' "$listing" | count_listed_tests)"

if [[ "$matched" -eq 0 ]]; then
    err "filter '$FILTER' matched no test in $PACKAGE"
    err "a filtered run that matches nothing exits 0; that is not evidence"
    exit 1
fi

info "$matched test(s) matched; running them"
bash "$SCRIPT_DIR/dev-test-host.sh" test -p "$PACKAGE" "$FILTER" --lib --locked --offline
