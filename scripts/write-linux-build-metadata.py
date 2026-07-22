#!/usr/bin/env python3
"""Capture the exact Linux SDK runtime toolchain and source provenance."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile


REVISION = re.compile(r"^[0-9a-fA-F]{40}$")
MIGO_LICENSES = ["Apache-2.0", "BSD-3-Clause", "BSL-1.1", "MIT"]


def command_output(command: list[str], label: str) -> str:
    try:
        result = subprocess.run(command, check=False, text=True, capture_output=True)
    except OSError as error:
        raise RuntimeError(f"cannot run {label}: {error}") from error
    if result.returncode != 0:
        raise RuntimeError(f"{label} failed: {result.stderr.strip()}")
    value = " | ".join(line.strip() for line in result.stdout.splitlines() if line.strip())
    if not value:
        raise RuntimeError(f"{label} returned an empty version")
    return value


def file_hash(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(64 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def source_revision(repo_root: pathlib.Path, explicit: str | None) -> str:
    value = explicit or os.environ.get("MIGO_SOURCE_REVISION") or os.environ.get("GITHUB_SHA")
    if not value:
        value = command_output(
            ["git", "-C", str(repo_root), "rev-parse", "HEAD"],
            "Migo source revision",
        )
    if not REVISION.fullmatch(value):
        raise RuntimeError("Migo source revision must be a full 40-character revision")
    return value


def repository_recipe(repo_root: pathlib.Path, value: str) -> tuple[str, pathlib.Path]:
    candidate = pathlib.PurePosixPath(value)
    if (
        candidate.is_absolute()
        or not candidate.parts
        or any(part in ("", ".", "..") for part in candidate.parts)
    ):
        raise RuntimeError("build recipe must be a safe repository-relative path")
    path = repo_root.joinpath(*candidate.parts)
    if not path.is_file():
        raise RuntimeError(f"build recipe does not exist: {path}")
    return candidate.as_posix(), path


def write_json_atomic(path: pathlib.Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary = pathlib.Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as output:
            json.dump(value, output, indent=2, sort_keys=True)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--compiler", required=True)
    parser.add_argument("--linker", required=True)
    parser.add_argument("--sysroot-identity", required=True)
    parser.add_argument("--source-revision")
    parser.add_argument("--build-recipe", default="scripts/build-linux-sdk.sh")
    arguments = parser.parse_args()

    try:
        repo_root = arguments.repo_root.resolve()
        recipe_name, recipe = repository_recipe(repo_root, arguments.build_recipe)
        if not arguments.sysroot_identity.strip():
            raise RuntimeError("sysroot identity must be non-empty")
        metadata = {
            "schema": "migo-linux-build-metadata/v1",
            "toolchain": {
                "rustc": command_output(["rustc", "--version", "--verbose"], "rustc"),
                "compiler": command_output([arguments.compiler, "--version"], "Linux C++ compiler"),
                "sdk": arguments.sysroot_identity,
                "linker": command_output([arguments.linker, "--version"], "Linux linker"),
            },
            "provenance": {
                "source_revision": source_revision(repo_root, arguments.source_revision),
                "build_recipe": recipe_name,
                "build_recipe_sha256": file_hash(recipe),
                "licenses": MIGO_LICENSES,
            },
        }
        write_json_atomic(arguments.output.resolve(), metadata)
        print(f"Linux build metadata -> {arguments.output.resolve()}")
    except (OSError, RuntimeError) as error:
        print(f"Linux build metadata: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
