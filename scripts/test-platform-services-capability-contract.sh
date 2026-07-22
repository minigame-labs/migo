#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT_DIR" <<'PY'
from __future__ import annotations

import pathlib
import re
import sys


root = pathlib.Path(sys.argv[1]).resolve()
errors: list[str] = []


def require(condition: bool, message: str) -> None:
    if not condition:
        errors.append(message)


platform_rs = (root / "engine/crates/core/src/services/platform.rs").read_text(encoding="utf-8")

for trait in ("DeviceServiceProvider", "FrameClock", "HostNotifier"):
    require(
        re.search(rf"\btrait\s+{trait}\b", platform_rs) is not None,
        f"core/services/platform.rs must declare capability trait `{trait}`",
    )

require(
    re.search(
        r"trait\s+PlatformServices\s*:\s*DeviceServiceProvider\s*\+\s*FrameClock\s*\+\s*HostNotifier",
        platform_rs,
    )
    is not None,
    "PlatformServices must be a supertrait of DeviceServiceProvider + FrameClock + HostNotifier",
)

require(
    re.search(
        r"impl\s*<\s*T\s*>\s*PlatformServices\s+for\s+T\b", platform_rs
    )
    is not None,
    "PlatformServices must have a blanket `impl<T> PlatformServices for T` so platforms implement the capability traits",
)


def marker_trait_has_no_methods(source: str) -> bool:
    """True if the `trait PlatformServices { ... }` body contains no `fn `."""
    idx = source.find("trait PlatformServices")
    if idx == -1:
        return False
    brace = source.find("{", idx)
    if brace == -1:
        # `trait PlatformServices: A + B + C {}` may be written with `{}`; if no
        # brace at all it is malformed.
        return False
    depth = 0
    body_start = brace
    for pos in range(brace, len(source)):
        ch = source[pos]
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                body = source[body_start + 1 : pos]
                return "fn " not in body
    return False


require(
    marker_trait_has_no_methods(platform_rs),
    "the `trait PlatformServices` block must be a marker (no inherent `fn` methods)",
)

android = (root / "engine/crates/platform/src/android/platform.rs").read_text(encoding="utf-8")
linux = (root / "engine/crates/platform/src/linux/platform.rs").read_text(encoding="utf-8")

for name, source in (("android", android), ("linux", linux)):
    require(
        "impl PlatformServices for" not in source,
        f"{name} platform must implement the capability traits, not `impl PlatformServices for`",
    )
    for trait in ("DeviceServiceProvider", "FrameClock", "HostNotifier"):
        require(
            f"impl {trait} for" in source,
            f"{name} platform must contain `impl {trait} for`",
        )

contract_step_name = "Platform services capability contract"
contract_command = "bash scripts/test-platform-services-capability-contract.sh"


def has_active_workflow_step(workflow: str) -> bool:
    lines = workflow.splitlines()
    expected_name = f"- name: {contract_step_name}"
    expected_run = f"run: {contract_command}"

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


active_fixture = f"""
steps:
  - name: {contract_step_name}
    run: {contract_command}
"""
require(
    has_active_workflow_step(active_fixture),
    "workflow checker must recognize an active capability contract step",
)
commented_fixture = f"""
steps:
  - name: {contract_step_name}
    # run: {contract_command}
"""
require(
    not has_active_workflow_step(commented_fixture),
    "workflow checker must reject a commented-out capability contract command",
)
for relative in (".github/workflows/pr-ci.yml", ".github/workflows/release.yml"):
    workflow = (root / relative).read_text(encoding="utf-8")
    require(
        has_active_workflow_step(workflow),
        f"{relative} must contain an active capability contract step",
    )

if errors:
    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    raise SystemExit(1)

print("Platform services capability contract: PASS")
PY
