#!/usr/bin/env python3
"""Compose verified Android per-slice manifests and their embedded package index."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
from typing import Any


SHA256 = re.compile(r"^[0-9a-f]{64}$")
REVISION = re.compile(r"^[0-9a-fA-F]{40}$")
ARCHITECTURES = {
    "arm64-v8a": {
        "arch": "aarch64",
        "triple": "aarch64-linux-android",
        "cpu_baseline": "armv8-a",
        "required_cpu_features": ["neon"],
    },
    "x86_64": {
        "arch": "x86_64",
        "triple": "x86_64-linux-android",
        "cpu_baseline": "x86-64-v1",
        "required_cpu_features": ["cmov", "sse2"],
    },
}


class ContractError(RuntimeError):
    pass


def byte_sorted(values: set[str] | list[str]) -> list[str]:
    return sorted(values, key=lambda value: value.encode("utf-8"))


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", required=True, type=pathlib.Path)
    parser.add_argument("--tool", required=True, type=pathlib.Path)
    parser.add_argument("--output-root", required=True, type=pathlib.Path)
    parser.add_argument("--build-metadata", required=True, type=pathlib.Path)
    parser.add_argument("--product-profile", required=True, choices=("full", "slim"))
    parser.add_argument("--build-type", required=True, choices=("debug", "release"))
    parser.add_argument("--codegen-profile", required=True, choices=("z", "2", "3"))
    parser.add_argument("--worker-snapshot", action="store_true")
    parser.add_argument(
        "--arch", action="append", required=True, choices=tuple(ARCHITECTURES)
    )
    return parser.parse_args()


def read_json(path: pathlib.Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as error:
        raise ContractError(f"missing {label}: {path}") from error
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ContractError(f"cannot read {label} {path}: {error}") from error
    if not isinstance(value, dict):
        raise ContractError(f"{label} must be a JSON object: {path}")
    return value


def require_exact_keys(value: dict[str, Any], expected: set[str], label: str) -> None:
    actual = set(value)
    if actual != expected:
        missing = sorted(expected - actual)
        unknown = sorted(actual - expected)
        raise ContractError(f"{label} fields differ (missing={missing}, unknown={unknown})")


def require_sha256(value: Any, label: str) -> str:
    if not isinstance(value, str) or not SHA256.fullmatch(value):
        raise ContractError(f"{label} must be a lowercase SHA-256")
    return value


def sha256_file(path: pathlib.Path, label: str) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(64 * 1024), b""):
                digest.update(chunk)
    except OSError as error:
        raise ContractError(f"cannot hash {label} {path}: {error}") from error
    return digest.hexdigest()


def verify_declared_file_hash(
    path: pathlib.Path, declared: Any, label: str
) -> str:
    declared_hash = require_sha256(declared, f"{label} hash")
    actual_hash = sha256_file(path, label)
    if actual_hash != declared_hash:
        raise ContractError(
            f"{label} hash mismatch (manifest={declared_hash}, computed={actual_hash})"
        )
    return actual_hash


def relative_recipe(repo_root: pathlib.Path, value: Any, label: str) -> pathlib.Path:
    if not isinstance(value, str) or not value or "\\" in value:
        raise ContractError(f"{label} must be a non-empty repository-relative path")
    path = pathlib.PurePosixPath(value)
    if path.is_absolute() or any(part in ("", ".", "..") for part in path.parts):
        raise ContractError(f"{label} must be a safe repository-relative path")
    return repo_root.joinpath(*path.parts)


def verify_recipe(
    repo_root: pathlib.Path, provenance: dict[str, Any], label: str
) -> None:
    recipe = relative_recipe(repo_root, provenance.get("build_recipe"), label)
    verify_declared_file_hash(
        recipe, provenance.get("build_recipe_sha256"), f"{label} bytes"
    )


def run_tool(tool: pathlib.Path, *arguments: os.PathLike[str] | str) -> str:
    command = [str(tool), *(str(argument) for argument in arguments)]
    try:
        result = subprocess.run(command, check=False, text=True, capture_output=True)
    except OSError as error:
        raise ContractError(f"cannot execute manifest tool {tool}: {error}") from error
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or "no diagnostic"
        raise ContractError(f"manifest tool rejected {' '.join(command[1:])}: {detail}")
    return result.stdout.strip()


def validate_build_metadata(
    repo_root: pathlib.Path, path: pathlib.Path
) -> dict[str, Any]:
    metadata = read_json(path, "Android build metadata")
    require_exact_keys(metadata, {"schema", "toolchain", "provenance"}, "build metadata")
    if metadata["schema"] != "migo-android-build-metadata/v1":
        raise ContractError("unsupported Android build metadata schema")
    toolchain = metadata["toolchain"]
    provenance = metadata["provenance"]
    if not isinstance(toolchain, dict) or not isinstance(provenance, dict):
        raise ContractError("build metadata toolchain/provenance must be objects")
    require_exact_keys(toolchain, {"rustc", "compiler", "sdk", "linker"}, "toolchain")
    require_exact_keys(
        provenance,
        {"source_revision", "build_recipe", "build_recipe_sha256", "licenses"},
        "provenance",
    )
    for field, value in toolchain.items():
        if not isinstance(value, str) or not value.strip():
            raise ContractError(f"toolchain.{field} must be non-empty")
    revision = provenance.get("source_revision")
    if not isinstance(revision, str) or not REVISION.fullmatch(revision):
        raise ContractError("provenance.source_revision must be a full revision")
    licenses = provenance.get("licenses")
    if (
        not isinstance(licenses, list)
        or not licenses
        or not all(isinstance(item, str) and item for item in licenses)
        or licenses != byte_sorted(set(licenses))
    ):
        raise ContractError("provenance.licenses must be sorted, unique, and non-empty")
    verify_recipe(repo_root, provenance, "Android build recipe")
    return metadata


def validate_component(
    repo_root: pathlib.Path,
    tool: pathlib.Path,
    abi: str,
    target: dict[str, Any],
) -> tuple[dict[str, Any], pathlib.Path, pathlib.Path]:
    component_root = repo_root / "engine/third_party/rusty_v8" / target["triple"]
    manifest_path = component_root / "component-manifest.json"
    archive_path = component_root / "librusty_v8.a"
    binding_path = component_root / "src_binding.rs"
    if not manifest_path.is_file():
        raise ContractError(
            f"missing verified V8 component-manifest.json for {abi}: {manifest_path}; "
            "rebuild it with scripts/build-v8-android.sh"
        )
    run_tool(tool, "verify-v8-component", manifest_path, archive_path, binding_path)
    component = read_json(manifest_path, f"{abi} V8 component manifest")
    component_target = component.get("target")
    if not isinstance(component_target, dict):
        raise ContractError(f"{abi} V8 component target is missing")
    expected = {
        "triple": target["triple"],
        "os": "android",
        "arch": target["arch"],
        "abi": "android",
        "cpu_baseline": target["cpu_baseline"],
        "required_cpu_features": target["required_cpu_features"],
        "runtime_floor": {"android_api": "26"},
    }
    if component_target != expected:
        raise ContractError(
            f"{abi} V8 component target does not match the Android API 26 contract"
        )
    provenance = component.get("provenance")
    if not isinstance(provenance, dict):
        raise ContractError(f"{abi} V8 component provenance is missing")
    verify_recipe(repo_root, provenance, f"{abi} V8 build recipe")
    return component, archive_path, binding_path


def validate_snapshot(
    repo_root: pathlib.Path,
    profile: str,
    kind: str,
    target: dict[str, Any],
    v8_archive_hash: str,
) -> dict[str, Any]:
    prefix = "SNAPSHOT" if kind == "host" else "SNAPSHOT-worker"
    name = f"{prefix}-{profile}-android-{target['arch']}.bin"
    snapshot_path = repo_root / "engine/crates/runtime-v8/snapshots" / name
    manifest_path = pathlib.Path(f"{snapshot_path}.manifest.json")
    manifest = read_json(manifest_path, f"{kind} snapshot manifest")
    required = {
        "schema_version",
        "snapshot_kind",
        "profile",
        "arch",
        "target_triple",
        "generation_cpu_policy",
        "normalized_parameters",
        "external_references_sha256",
        "bootstrap_inputs_sha256",
        "v8_archive_sha256",
        "snapshot_size",
        "snapshot_sha256",
    }
    missing = sorted(required - set(manifest))
    if missing:
        raise ContractError(
            f"{kind} snapshot manifest {manifest_path} lacks verified v1 fields: {missing}; "
            "regenerate the snapshot"
        )
    expected_parameters = byte_sorted(
        [
            f"--arch={target['arch']}",
            "--cpu-policy=target-baseline",
            f"--product-profile={profile}",
            f"--runtime-kind={kind}",
            "--warmup=none",
        ]
    )
    expected_identity = {
        "schema_version": 3,
        "snapshot_kind": kind,
        "profile": profile,
        "arch": target["arch"],
        "target_triple": target["triple"],
        "generation_cpu_policy": "target-baseline",
        "normalized_parameters": expected_parameters,
    }
    for field, expected in expected_identity.items():
        if manifest.get(field) != expected:
            raise ContractError(
                f"{kind} snapshot {field} mismatch (manifest={manifest.get(field)!r}, "
                f"expected={expected!r})"
            )
    if manifest.get("v8_archive_sha256") != v8_archive_hash:
        raise ContractError(f"{kind} snapshot was generated for a different V8 archive")
    snapshot_hash = verify_declared_file_hash(
        snapshot_path, manifest.get("snapshot_sha256"), f"{kind} snapshot"
    )
    try:
        declared_size = int(manifest.get("snapshot_size"))
    except (TypeError, ValueError) as error:
        raise ContractError(f"{kind} snapshot_size must be an integer") from error
    if snapshot_path.stat().st_size != declared_size:
        raise ContractError(f"{kind} snapshot_size does not match its bytes")
    return {
        "runtime_kind": kind,
        "product_profile": profile,
        "target_triple": target["triple"],
        "arch": target["arch"],
        "schema": str(manifest["schema_version"]),
        "generator": "migo-snapshot-gen/schema-v3",
        "generation_cpu_policy": manifest["generation_cpu_policy"],
        "normalized_parameters": manifest["normalized_parameters"],
        "external_references_hash": require_sha256(
            manifest["external_references_sha256"],
            f"{kind} snapshot external references",
        ),
        "bootstrap_inputs_hash": require_sha256(
            manifest["bootstrap_inputs_sha256"], f"{kind} snapshot bootstrap inputs"
        ),
        "bytes_hash": snapshot_hash,
    }


def combined_toolchain(
    runtime: dict[str, Any], v8: dict[str, Any]
) -> dict[str, str]:
    output: dict[str, str] = {}
    for field in ("rustc", "compiler", "sdk", "linker"):
        output[field] = f"runtime={runtime[field]}; v8={v8[field]}"
    return output


def generate(arguments: argparse.Namespace) -> None:
    repo_root = arguments.repo_root.resolve()
    tool = arguments.tool.resolve()
    output_root = arguments.output_root.resolve()
    if tuple(output_root.parts[-3:]) != ("assets", "migo", "artifacts"):
        raise ContractError("output root must end with assets/migo/artifacts")
    if not tool.is_file():
        raise ContractError(f"manifest tool does not exist: {tool}")
    if arguments.build_type == "debug" and arguments.codegen_profile != "z":
        raise ContractError("debug artifacts require codegen profile z")
    if arguments.worker_snapshot and (
        arguments.build_type != "release" or arguments.product_profile != "full"
    ):
        raise ContractError("Worker snapshot requires a full release artifact")
    if len(set(arguments.arch)) != len(arguments.arch):
        raise ContractError("duplicate --arch values are forbidden")

    metadata = validate_build_metadata(repo_root, arguments.build_metadata.resolve())
    output_root.parent.mkdir(parents=True, exist_ok=True)
    staging_parent = output_root.parents[2]
    staging_parent.mkdir(parents=True, exist_ok=True)
    staging = pathlib.Path(
        tempfile.mkdtemp(prefix=".migo-artifacts.", dir=staging_parent)
    )
    try:
        slices_root = staging / "slices"
        slices_root.mkdir(parents=True)
        index_sources: list[str] = []
        codegen_suffix = (
            "" if arguments.codegen_profile == "z" else f"-opt{arguments.codegen_profile}"
        )
        worker_suffix = "-worker-snapshot" if arguments.worker_snapshot else ""
        native_root = repo_root / "engine/jniLibs" / (
            f"{arguments.product_profile}{codegen_suffix}{worker_suffix}"
        )

        for abi in arguments.arch:
            target = ARCHITECTURES[abi]
            component, archive_path, binding_path = validate_component(
                repo_root, tool, abi, target
            )
            archive_hash = sha256_file(archive_path, f"{abi} V8 archive")
            snapshots = [
                validate_snapshot(
                    repo_root,
                    arguments.product_profile,
                    "host",
                    target,
                    archive_hash,
                )
            ]
            if arguments.worker_snapshot:
                snapshots.append(
                    validate_snapshot(
                        repo_root,
                        arguments.product_profile,
                        "worker",
                        target,
                        archive_hash,
                    )
                )

            runtime_binary = native_root / abi / "libmigo.so"
            cxx_runtime = native_root / abi / "libc++_shared.so"
            slice_manifest = {
                "schema": "migo-artifact-manifest/v1",
                "artifact_id": "",
                "product_profile": arguments.product_profile,
                "build_type": arguments.build_type,
                "codegen_profile": arguments.codegen_profile,
                "target": component["target"],
                "toolchain": combined_toolchain(
                    metadata["toolchain"], component["toolchain"]
                ),
                "runtime": component["runtime"],
                "snapshots": snapshots,
                "graphics": {
                    "backend_family": "gles-native",
                    "required_api": "OpenGL ES 3.0",
                },
                "hashes": {
                    "runtime_binary": sha256_file(runtime_binary, f"{abi} libmigo.so"),
                    "v8_archive": archive_hash,
                    "rust_binding": sha256_file(binding_path, f"{abi} V8 binding"),
                    "cxx_runtime": sha256_file(cxx_runtime, f"{abi} libc++ runtime"),
                },
                "provenance": metadata["provenance"],
            }
            draft_path = staging / f".{abi}.input.json"
            final_path = slices_root / f"{abi}.json"
            draft_path.write_text(
                json.dumps(slice_manifest, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
            try:
                run_tool(tool, "seal-slice", draft_path, final_path)
                run_tool(tool, "verify-slice", final_path)
            finally:
                draft_path.unlink(missing_ok=True)
            package_path = f"assets/migo/artifacts/slices/{abi}.json"
            index_sources.append(f"{package_path}={final_path}")

        run_tool(
            tool,
            "build-index",
            arguments.product_profile,
            staging / "package-index.json",
            *index_sources,
        )
        if output_root.exists():
            shutil.rmtree(output_root)
        os.replace(staging, output_root)
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise

    print(f"Android artifact manifests -> {output_root}")


def main() -> int:
    try:
        generate(parse_arguments())
    except ContractError as error:
        print(f"artifact manifest generation: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
