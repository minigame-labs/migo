#!/usr/bin/env python3
"""Which of the workspace's test binaries no host step runs.

`scripts/verify-change.sh` declares its host suites as a list of cargo step
strings. Every one of them said `--lib`, on both sides of the gate, and nothing
compared that list against the targets cargo would actually build -- so thirteen
integration-test binaries holding 95 tests were run by no local step, and 35 of
them by no job anywhere. `migo-capi-abi`'s nine binaries are the sharpest case:
its lib has no tests at all, so `--lib` there is a step that cannot fail.

The audit exists because the crate-name comparison in
`scripts/test-local-verification-contract.sh` could not see this. Two lists that
name the same crates can still run different binaries, and the difference is one
word in the middle of a step.

Reads the steps on stdin, one per line, and prints `package::target` for every
`kind: ["test"]` target no step runs.

Exit codes:
    0  coverage determined; uncovered targets, if any, are on stdout
    1  cargo could not describe the workspace, so coverage is unknown
    3  no `engine/Cargo.toml` in this tree, so there is nothing to audit
"""

from __future__ import annotations

import argparse
import json
import pathlib
import shlex
import subprocess
import sys

# Target-selection flags that include a package's integration tests.
_INCLUDES_TESTS = {"--tests", "--all-targets"}

# Flags selecting some other target kind. A step carrying only these runs no
# integration test, which is exactly what `--lib` did everywhere.
_SELECTS_OTHER = {"--lib", "--bins", "--doc", "--examples", "--benches"}

# Flags that consume the following word, so it is never mistaken for a flag.
_TAKES_VALUE = {"--test", "--bin", "--example", "--bench", "-p", "--package",
                "--features", "--target", "--manifest-path", "-j", "--jobs"}

ALL = object()
"""Sentinel: this step runs every test target its packages have."""


def step_coverage(step: str) -> tuple[list[str], object | set[str]] | None:
    """The packages a step names and the test targets it runs, or None if it is
    not a `cargo test` step at all.

    `build --workspace --all-targets` is deliberately None: it compiles every
    test binary and runs none of them, and treating a compile as coverage is how
    a binary comes to exist in a green tree without ever executing.
    """
    words = shlex.split(step)
    if not words or words[0] != "test":
        return None

    packages: list[str] = []
    named: set[str] = set()
    includes_tests = False
    selects_other = False

    index = 1
    while index < len(words):
        word = words[index]
        if word in ("-p", "--package"):
            index += 2
            if index - 1 < len(words):
                packages.append(words[index - 1])
            continue
        if word == "--test":
            index += 2
            if index - 1 < len(words):
                named.add(words[index - 1])
            continue
        if word in _INCLUDES_TESTS:
            includes_tests = True
        elif word in _SELECTS_OTHER:
            selects_other = True
        elif word in _TAKES_VALUE:
            index += 1
        index += 1

    if includes_tests:
        return packages, ALL
    if named:
        return packages, named
    if selects_other:
        return packages, set()
    # No target selection at all: cargo runs the lib, the integration tests and
    # the doc tests.
    return packages, ALL


def coverage_by_package(steps) -> dict[str, object | set[str]]:
    merged: dict[str, object | set[str]] = {}
    for step in steps:
        parsed = step_coverage(step)
        if parsed is None:
            continue
        packages, covered = parsed
        for package in packages:
            existing = merged.get(package)
            if existing is ALL or covered is ALL:
                merged[package] = ALL
            else:
                merged[package] = set(existing or set()) | set(covered)
    return merged


def test_targets(engine: pathlib.Path) -> dict[str, list[str]]:
    """Every workspace member's `kind: ["test"]` targets, from cargo itself.

    Globbing `tests/*.rs` would be a second implementation of cargo's target
    discovery: it counts `tests/common/mod.rs` as a binary when it is a module
    directory, and it cannot see an explicit `[[test]]`.
    """
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--offline", "--format-version", "1"],
        cwd=engine,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "cargo metadata failed")
    metadata = json.loads(result.stdout)

    targets: dict[str, list[str]] = {}
    for package in metadata.get("packages", []):
        names = [
            target["name"]
            for target in package.get("targets", [])
            if target.get("kind") == ["test"]
        ]
        if names:
            targets[package["name"]] = sorted(names)
    return targets


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, help="repository root")
    args = parser.parse_args()

    engine = pathlib.Path(args.root) / "engine"
    if not (engine / "Cargo.toml").is_file():
        return 3

    try:
        targets = test_targets(engine)
    except (RuntimeError, json.JSONDecodeError, OSError) as error:
        print(f"cargo could not describe the workspace: {error}", file=sys.stderr)
        return 1

    covered = coverage_by_package(
        line.strip() for line in sys.stdin if line.strip()
    )

    for package in sorted(targets):
        reach = covered.get(package)
        for target in targets[package]:
            if reach is ALL or (reach is not None and target in reach):
                continue
            print(f"{package}::{target}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
