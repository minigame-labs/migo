#!/usr/bin/env python3
"""Capture the exact final Android runtime toolchain for slice composition."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import platform
import re
import subprocess
import sys
import tempfile


REVISION = re.compile(r"^[0-9a-fA-F]{40}$")


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


def find_prebuilt(ndk: pathlib.Path) -> pathlib.Path:
    root = ndk / "toolchains/llvm/prebuilt"
    system = platform.system().lower()
    machine = platform.machine().lower()
    aliases = {
        ("linux", "amd64"): "linux-x86_64",
        ("linux", "x86_64"): "linux-x86_64",
        ("linux", "aarch64"): "linux-aarch64",
        ("darwin", "arm64"): "darwin-arm64",
        ("darwin", "aarch64"): "darwin-arm64",
        ("darwin", "x86_64"): "darwin-x86_64",
    }
    preferred = aliases.get((system, machine))
    if preferred and (root / preferred).is_dir():
        return root / preferred
    candidates = sorted(path for path in root.glob("*") if path.is_dir())
    if len(candidates) != 1:
        raise RuntimeError(f"cannot select a unique NDK host prebuilt under {root}")
    return candidates[0]


def source_revision(repo_root: pathlib.Path, explicit: str | None) -> str:
    value = explicit or os.environ.get("MIGO_SOURCE_REVISION") or os.environ.get("GITHUB_SHA")
    if not value:
        value = command_output(
            ["git", "-C", str(repo_root), "rev-parse", "HEAD"], "Migo source revision"
        )
    if not REVISION.fullmatch(value):
        raise RuntimeError("Migo source revision must be a full 40-character revision")
    return value


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
    parser.add_argument("--ndk-home", type=pathlib.Path)
    parser.add_argument("--source-revision")
    arguments = parser.parse_args()

    try:
        repo_root = arguments.repo_root.resolve()
        ndk_value = arguments.ndk_home or (
            pathlib.Path(os.environ["ANDROID_NDK_HOME"])
            if os.environ.get("ANDROID_NDK_HOME")
            else None
        )
        if ndk_value is None:
            raise RuntimeError("ANDROID_NDK_HOME is required")
        ndk = ndk_value.resolve()
        properties = ndk / "source.properties"
        text = properties.read_text(encoding="utf-8")
        match = re.search(r"(?m)^Pkg\.Revision\s*=\s*(\S+)\s*$", text)
        if not match:
            raise RuntimeError(f"NDK revision is missing from {properties}")
        prebuilt = find_prebuilt(ndk)
        clang = prebuilt / "bin/aarch64-linux-android26-clang++"
        if not clang.is_file():
            clang = prebuilt / "bin/clang++"
        linker = prebuilt / "bin/ld.lld"
        recipe = repo_root / "scripts/build-aar.sh"
        metadata = {
            "schema": "migo-android-build-metadata/v1",
            "toolchain": {
                "rustc": command_output(["rustc", "--version", "--verbose"], "rustc"),
                "compiler": command_output([str(clang), "--version"], "Android clang"),
                "sdk": (
                    f"Android NDK {match.group(1)}; API 26 sysroot; "
                    f"source.properties sha256={file_hash(properties)}"
                ),
                "linker": command_output([str(linker), "--version"], "Android linker"),
            },
            "provenance": {
                "source_revision": source_revision(repo_root, arguments.source_revision),
                "build_recipe": "scripts/build-aar.sh",
                "build_recipe_sha256": file_hash(recipe),
                "licenses": ["Apache-2.0", "BSD-3-Clause", "MIT"],
            },
        }
        write_json_atomic(arguments.output.resolve(), metadata)
        print(f"Android build metadata -> {arguments.output.resolve()}")
    except (OSError, RuntimeError) as error:
        print(f"Android build metadata: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
