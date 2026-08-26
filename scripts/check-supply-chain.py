#!/usr/bin/env python3
"""Validate immutable CI inputs, Rust advisories, sources, and licenses."""

from __future__ import annotations

import argparse
import datetime as dt
import pathlib
import sys

from lib.supply_chain import (
    PolicyError,
    load_policy,
    read_json,
    validate_actions,
    validate_audit,
    validate_gradle,
    validate_metadata,
)


def actions_command(args: argparse.Namespace) -> int:
    count, errors = validate_actions(args.workflows_dir.resolve(), args.exclude)
    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        print(f"Supply-chain Actions gate: FAIL ({len(errors)} violation(s))", file=sys.stderr)
        return 1
    print(f"Supply-chain Actions gate: PASS ({count} immutable remote reference(s))")
    return 0


def audit_command(args: argparse.Namespace) -> int:
    policy = load_policy(args.policy.resolve())
    # Keep /dev/fd process-substitution paths intact. Resolving them first turns
    # the descriptor symlink into a short-lived /proc pipe name that cannot be
    # reopened, which makes the no-temporary-file CI invocation fail.
    audit = read_json(args.audit_json)
    metadata = read_json(args.metadata_json)
    as_of = dt.date.fromisoformat(args.as_of) if args.as_of else dt.date.today()
    warning_count, errors = validate_audit(audit, policy, as_of)
    licenses, metadata_errors = validate_metadata(
        metadata, policy, args.workspace_root.resolve()
    )
    errors.extend(metadata_errors)
    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        print(f"Rust supply-chain gate: FAIL ({len(errors)} violation(s))", file=sys.stderr)
        return 1
    print(
        "Rust supply-chain gate: PASS "
        f"({len(licenses)} package license(s), {warning_count} controlled warning(s))"
    )
    return 0


def gradle_command(args: argparse.Namespace) -> int:
    policy = load_policy(args.policy.resolve())
    locked, checksums, errors = validate_gradle(args.project_dir.resolve(), policy)
    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        print(f"Gradle supply-chain gate: FAIL ({len(errors)} violation(s))", file=sys.stderr)
        return 1
    print(
        f"Gradle supply-chain gate: PASS ({locked} locked module(s), "
        f"{checksums} verified artifact checksum(s))"
    )
    return 0


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    subparsers = result.add_subparsers(dest="command", required=True)
    actions = subparsers.add_parser("actions", help="require immutable action revisions")
    actions.add_argument("--workflows-dir", type=pathlib.Path, required=True)
    actions.add_argument("--exclude", action="append", default=[])
    actions.set_defaults(run=actions_command)

    audit = subparsers.add_parser("audit", help="enforce Rust advisory/source/license policy")
    audit.add_argument("--audit-json", type=pathlib.Path, required=True)
    audit.add_argument("--metadata-json", type=pathlib.Path, required=True)
    audit.add_argument("--policy", type=pathlib.Path, required=True)
    audit.add_argument("--workspace-root", type=pathlib.Path, required=True)
    audit.add_argument("--as-of", help="policy date in YYYY-MM-DD (defaults to today)")
    audit.set_defaults(run=audit_command)

    gradle = subparsers.add_parser("gradle", help="enforce Gradle wrapper/lock/checksum policy")
    gradle.add_argument("--project-dir", type=pathlib.Path, required=True)
    gradle.add_argument("--policy", type=pathlib.Path, required=True)
    gradle.set_defaults(run=gradle_command)
    return result


def main() -> int:
    args = parser().parse_args()
    try:
        return int(args.run(args))
    except (PolicyError, ValueError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
