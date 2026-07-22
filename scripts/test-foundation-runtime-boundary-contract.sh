#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT_DIR" <<'PY'
from __future__ import annotations

import pathlib
import sys
import tomllib


root = pathlib.Path(sys.argv[1]).resolve()
errors: list[str] = []


def require(condition: bool, message: str) -> None:
    if not condition:
        errors.append(message)


def dependency_names(node: object) -> set[str]:
    names: set[str] = set()
    if not isinstance(node, dict):
        return names
    for key, value in node.items():
        if key in {"dependencies", "dev-dependencies", "build-dependencies"}:
            if isinstance(value, dict):
                for alias, specification in value.items():
                    names.add(alias)
                    if isinstance(specification, dict):
                        package = specification.get("package")
                        if isinstance(package, str):
                            names.add(package)
            continue
        names.update(dependency_names(value))
    return names


# `shared` and `io` define engine data/persistence services. Pulling a JS
# runtime through either crate makes every consumer execute rusty_v8's build
# script and prevents alternate runtime backends from reusing the foundation.
# JSON is ordinary wire/storage data here; use serde_json directly.
for crate in ("shared", "io"):
    crate_root = root / "engine/crates" / crate
    manifest = tomllib.loads((crate_root / "Cargo.toml").read_text(encoding="utf-8"))
    dependencies = dependency_names(manifest)
    for forbidden in ("deno_core", "deno_error", "v8", "migo-runtime-v8"):
        require(
            forbidden not in dependencies,
            f"migo-{crate} must not directly depend on runtime package {forbidden}",
        )
    require(
        "serde_json" in manifest.get("dependencies", {}),
        f"migo-{crate} must use serde_json directly for JSON data",
    )
    for source_path in sorted((crate_root / "src").rglob("*.rs")):
        source = source_path.read_text(encoding="utf-8")
        relative = source_path.relative_to(root)
        require(
            "deno_core" not in source and "deno_error" not in source,
            f"foundation source must not name a JS runtime crate: {relative}",
        )


step_name = "Foundation/runtime dependency boundary contract"
step_command = "bash scripts/test-foundation-runtime-boundary-contract.sh"


def has_active_workflow_step(workflow: str) -> bool:
    lines = workflow.splitlines()
    expected_name = f"- name: {step_name}"
    expected_run = f"run: {step_command}"
    for index, line in enumerate(lines):
        if line.strip() != expected_name:
            continue
        step_indent = len(line) - len(line.lstrip(" "))
        for candidate in lines[index + 1 :]:
            stripped = candidate.strip()
            if not stripped or stripped.startswith("#"):
                continue
            candidate_indent = len(candidate) - len(candidate.lstrip(" "))
            if candidate_indent <= step_indent and stripped.startswith("- "):
                break
            if candidate_indent == step_indent + 2 and stripped == expected_run:
                return True
    return False


for relative in (".github/workflows/pr-ci.yml", ".github/workflows/release.yml"):
    workflow = (root / relative).read_text(encoding="utf-8")
    require(
        has_active_workflow_step(workflow),
        f"{relative} must contain the active foundation/runtime boundary gate",
    )

if errors:
    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    raise SystemExit(1)

print("Foundation/runtime dependency boundary contract: PASS")
PY
