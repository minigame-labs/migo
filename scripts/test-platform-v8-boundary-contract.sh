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


platform_manifest_path = root / "engine/crates/platform/Cargo.toml"
platform_manifest = tomllib.loads(platform_manifest_path.read_text(encoding="utf-8"))


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


direct_dependencies = dependency_names(platform_manifest)

# The V8 backend package has been renamed once already (`js-runtime` ->
# `migo-runtime-v8`). A forbidden list that names a package which no longer
# exists cannot fail: the gate goes green by looking for the wrong string. So
# the name is asserted against the workspace before it is used as a rule.
V8_BACKEND_PACKAGE = "migo-runtime-v8"
backend_manifest_path = root / "engine/crates/runtime-v8/Cargo.toml"
require(
    backend_manifest_path.is_file(),
    f"V8 backend manifest not found at {backend_manifest_path}",
)
if backend_manifest_path.is_file():
    backend_manifest = tomllib.loads(backend_manifest_path.read_text(encoding="utf-8"))
    declared_name = backend_manifest.get("package", {}).get("name")
    require(
        declared_name == V8_BACKEND_PACKAGE,
        "this gate forbids a package name that the workspace no longer uses "
        f"(forbidding {V8_BACKEND_PACKAGE!r}, workspace declares {declared_name!r}); "
        "update V8_BACKEND_PACKAGE or the gate silently passes",
    )

aliased_runtime_fixture = {
    "dependencies": {
        "runtime_backend": {
            "package": V8_BACKEND_PACKAGE,
            "path": "../runtime-v8",
        }
    }
}
require(
    V8_BACKEND_PACKAGE in dependency_names(aliased_runtime_fixture),
    "boundary checker must resolve Cargo dependency aliases to package identity",
)
for forbidden in (V8_BACKEND_PACKAGE, "deno_core", "deno_error"):
    require(
        forbidden not in direct_dependencies,
        f"platform Cargo.toml must not directly depend on {forbidden}",
    )
require(
    "serde_json" in platform_manifest.get("dependencies", {}),
    "platform Cargo.toml must declare serde_json directly",
)

features = platform_manifest.get("features", {})
for feature, values in features.items():
    for value in values:
        require(
            not value.startswith(f"{V8_BACKEND_PACKAGE}/"),
            f"platform feature {feature!r} must forward runtime features through core, got {value!r}",
        )

platform_root = root / "engine/crates/platform"
for source_path in sorted(platform_root.rglob("*.rs")):
    source = source_path.read_text(encoding="utf-8")
    require(
        "deno_core" not in source,
        f"platform source must not name deno_core: {source_path.relative_to(root)}",
    )

platform_service_path = root / "engine/crates/core/src/services/platform.rs"
platform_service = platform_service_path.read_text(encoding="utf-8")
require(
    "deno_core" not in platform_service,
    "PlatformServices must not import or expose deno_core",
)
require(
    "fn extensions(" not in platform_service,
    "PlatformServices must not expose an extensions method",
)
require(
    "Vec<Extension>" not in platform_service,
    "PlatformServices must not expose deno_core::Extension values",
)

for relative in (
    "engine/crates/platform/src/android/platform.rs",
    "engine/crates/platform/src/linux/platform.rs",
):
    source = (root / relative).read_text(encoding="utf-8")
    require(
        "fn extensions(" not in source,
        f"platform implementation must not manufacture runtime extensions: {relative}",
    )

host_path = root / "engine/crates/core/src/runtime/host.rs"
host = host_path.read_text(encoding="utf-8")
require(
    "platform.extensions(" not in host,
    "core Host must not request runtime extensions from PlatformServices",
)
require(
    "let extra_ext" not in host,
    "core Host must not stage platform V8 extensions",
)

host_runtime_path = root / "engine/crates/runtime-v8/src/host_runtime.rs"
host_runtime = host_runtime_path.read_text(encoding="utf-8")
require(
    "extra_extensions" not in host_runtime,
    "HostJsRuntime::new must own extension assembly without an extra_extensions input",
)
require(
    ".chain(extra_extensions)" not in host_runtime,
    "V8 extension assembly must not chain platform-provided extensions",
)
require(
    "crate::snapshot::lazy_extensions()" in host_runtime
    and "main_extensions(host_state.clone())" in host_runtime,
    "snapshot and source extension assembly must remain explicit in HostJsRuntime",
)

contract_step_name = "Platform/V8 dependency boundary contract"
contract_command = "bash scripts/test-platform-v8-boundary-contract.sh"


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


active_workflow_fixture = f"""
steps:
  - name: {contract_step_name}
    run: {contract_command}
"""
require(
    has_active_workflow_step(active_workflow_fixture),
    "workflow checker must recognize an active Platform/V8 contract step",
)


commented_workflow_fixture = f"""
steps:
  - name: {contract_step_name}
    # run: {contract_command}
"""
require(
    not has_active_workflow_step(commented_workflow_fixture),
    "workflow checker must reject a commented-out Platform/V8 contract command",
)
for relative in (".github/workflows/pr-ci.yml", ".github/workflows/release.yml"):
    workflow = (root / relative).read_text(encoding="utf-8")
    require(
        has_active_workflow_step(workflow),
        f"{relative} must contain an active Platform/V8 boundary contract step",
    )

architecture = (root / "docs/multiplatform-architecture.md").read_text(encoding="utf-8")
require(
    "Platform/V8 Phase A 已完成" in architecture,
    "architecture status must record Platform/V8 Phase A completion",
)
require(
    "无 `JsBackend` trait" in architecture,
    "architecture status must say that full runtime-backend abstraction remains incomplete",
)

if errors:
    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    raise SystemExit(1)

print("Platform/V8 dependency boundary contract: PASS")
PY
