#!/usr/bin/env python3
"""Emit a verified Android V8 component manifest after a source build."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import platform
import re
import shlex
import subprocess
import sys
import tempfile


PATCH_FILES = {
    "0001-unset-bindgen-extra-clang-args": "0001-unset-BINDGEN_EXTRA_CLANG_ARGS-in-v8_s-bindgen.patch",
    "0002-use-sysroot-on-android": "0002-install-sysroot.patch",
    "0003-custom-libcxx-for-snapshot-toolchain": "0003-compiler-use-custom-libcxx-for-v8.patch",
}


def byte_sorted(values: set[str] | list[str]) -> list[str]:
    return sorted(values, key=lambda value: value.encode("utf-8"))


def run(command: list[str], label: str, *, allow_empty: bool = False) -> str:
    try:
        result = subprocess.run(command, check=False, text=True, capture_output=True)
    except OSError as error:
        raise RuntimeError(f"cannot run {label}: {error}") from error
    if result.returncode != 0:
        raise RuntimeError(f"{label} failed: {result.stderr.strip()}")
    output = " | ".join(line.strip() for line in result.stdout.splitlines() if line.strip())
    if not output and not allow_empty:
        raise RuntimeError(f"{label} returned empty output")
    return output


def hash_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(64 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git_revision(path: pathlib.Path, label: str) -> str:
    value = run(["git", "-C", str(path), "rev-parse", "HEAD"], label)
    if not re.fullmatch(r"[0-9a-fA-F]{40}", value):
        raise RuntimeError(f"{label} is not a full revision: {value!r}")
    return value


def package_version(cargo_toml: pathlib.Path) -> str:
    in_package = False
    for line in cargo_toml.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            in_package = stripped == "[package]"
        elif in_package:
            match = re.fullmatch(r'version\s*=\s*"([^"]+)"', stripped)
            if match:
                return match.group(1)
    raise RuntimeError(f"cannot find [package] version in {cargo_toml}")


def find_prebuilt(ndk: pathlib.Path) -> pathlib.Path:
    root = ndk / "toolchains/llvm/prebuilt"
    key = (platform.system().lower(), platform.machine().lower())
    names = {
        ("linux", "x86_64"): "linux-x86_64",
        ("linux", "amd64"): "linux-x86_64",
        ("linux", "aarch64"): "linux-aarch64",
        ("darwin", "arm64"): "darwin-arm64",
        ("darwin", "aarch64"): "darwin-arm64",
        ("darwin", "x86_64"): "darwin-x86_64",
    }
    preferred = names.get(key)
    if preferred and (root / preferred).is_dir():
        return root / preferred
    candidates = sorted(path for path in root.glob("*") if path.is_dir())
    if len(candidates) != 1:
        raise RuntimeError(f"cannot select a unique NDK prebuilt under {root}")
    return candidates[0]


def git_status(path: pathlib.Path, label: str) -> list[tuple[str, str]]:
    try:
        result = subprocess.run(
            [
                "git",
                "-C",
                str(path),
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
            ],
            check=False,
            text=True,
            capture_output=True,
        )
    except OSError as error:
        raise RuntimeError(f"cannot run {label}: {error}") from error
    if result.returncode != 0:
        raise RuntimeError(f"{label} failed: {result.stderr.strip()}")

    entries: list[tuple[str, str]] = []
    for record in result.stdout.split("\0"):
        if not record:
            continue
        if len(record) < 4 or record[2] != " ":
            entries.append(("invalid", record))
        else:
            entries.append((record[:2], record[3:]))
    return entries


def check_allowed_changes(
    path: pathlib.Path, label: str, allowed_paths: set[str]
) -> None:
    allowed_statuses = {" M", "M ", "MM"}
    unexpected = [
        f"{status} {changed_path}"
        for status, changed_path in git_status(path, f"{label} status")
        if changed_path not in allowed_paths or status not in allowed_statuses
    ]
    if unexpected:
        raise RuntimeError(
            f"{label} has unrelated tracked or untracked changes: {unexpected}"
        )


def check_source_changes(source: pathlib.Path) -> None:
    build_source = source / "build"
    nested_build = (build_source / ".git").exists()
    top_allowed = {"build.rs"}
    if nested_build:
        top_allowed.add("build")
    else:
        top_allowed.update(
            {
                "build/rust/gni_impl/run_bindgen.py",
                "build/config/c++/c++.gni",
            }
        )
    check_allowed_changes(source, "rusty_v8 source", top_allowed)
    check_allowed_changes(source / "v8", "V8 source", set())
    if nested_build:
        check_allowed_changes(
            build_source,
            "Chromium build source",
            {
                "rust/gni_impl/run_bindgen.py",
                "config/c++/c++.gni",
            },
        )


def normalized_gn_arguments(value: str, api: int) -> list[str]:
    arguments = []
    keys: set[str] = set()
    for argument in shlex.split(value):
        if argument.startswith("android_ndk_root="):
            argument = "android_ndk_root=${ANDROID_NDK_HOME}"
        if "=" not in argument or not argument.split("=", 1)[0]:
            raise RuntimeError(f"GN arg must use key=value syntax: {argument!r}")
        key = argument.split("=", 1)[0]
        if key in keys:
            raise RuntimeError(f"duplicate GN argument key: {key}")
        keys.add(key)
        arguments.append(argument)
    arguments = byte_sorted(set(arguments))
    if f"android_ndk_api_level={api}" not in arguments:
        raise RuntimeError(f"GN args do not pin android_ndk_api_level={api}")
    return arguments


def write_draft(path: pathlib.Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", required=True, type=pathlib.Path)
    parser.add_argument("--rusty-v8-src", required=True, type=pathlib.Path)
    parser.add_argument("--ndk-home", required=True, type=pathlib.Path)
    parser.add_argument("--arch", required=True, choices=("aarch64", "x86_64"))
    parser.add_argument("--extra-gn-args", required=True)
    parser.add_argument("--archive", required=True, type=pathlib.Path)
    parser.add_argument("--binding", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--tool", type=pathlib.Path)
    parser.add_argument("--lock", type=pathlib.Path)
    arguments = parser.parse_args()

    try:
        repo = arguments.repo_root.resolve()
        source = arguments.rusty_v8_src.resolve()
        ndk = arguments.ndk_home.resolve()
        lock_path = arguments.lock or repo / "contracts/artifact-manifest/android-v8.lock.json"
        lock = json.loads(lock_path.read_text(encoding="utf-8"))
        if lock.get("schema") != "migo-v8-build-lock/v1":
            raise RuntimeError(f"unsupported V8 source lock: {lock_path}")
        source_revision = git_revision(source, "rusty_v8 revision")
        v8_revision = git_revision(source / "v8", "V8 revision")
        version = package_version(source / "Cargo.toml")
        if source_revision != lock.get("rusty_v8_revision"):
            raise RuntimeError("rusty_v8 source revision does not match android-v8.lock.json")
        if v8_revision != lock.get("v8_revision"):
            raise RuntimeError("V8 source revision does not match android-v8.lock.json")
        if version != lock.get("rusty_v8_version"):
            raise RuntimeError("rusty_v8 version does not match android-v8.lock.json")
        check_source_changes(source)

        api = lock.get("android_api")
        target = lock.get("targets", {}).get(arguments.arch)
        if api != 26 or not isinstance(target, dict):
            raise RuntimeError("V8 lock does not contain the Android API 26 target")
        required_patches = lock.get("required_patches")
        if required_patches != list(PATCH_FILES):
            raise RuntimeError("V8 lock patch set/order differs from the supported build recipe")
        patch_root = repo / "engine/third_party/v8-patches"
        patches = []
        for patch_id in required_patches:
            patch_path = patch_root / PATCH_FILES[patch_id]
            patches.append({"id": patch_id, "sha256": hash_file(patch_path)})

        properties = ndk / "source.properties"
        properties_text = properties.read_text(encoding="utf-8")
        match = re.search(r"(?m)^Pkg\.Revision\s*=\s*(\S+)\s*$", properties_text)
        if not match:
            raise RuntimeError(f"NDK revision is missing from {properties}")
        prebuilt = find_prebuilt(ndk)
        clang = prebuilt / "bin" / f"{target['triple']}{api}-clang++"
        if not clang.is_file():
            clang = prebuilt / "bin/clang++"
        linker = prebuilt / "bin/ld.lld"
        recipe = repo / "scripts/build-v8-android.sh"
        component = {
            "schema": "migo-v8-component-manifest/v1",
            "component_id": "",
            "target": {
                "triple": target["triple"],
                "os": "android",
                "arch": arguments.arch,
                "abi": "android",
                "cpu_baseline": target["cpu_baseline"],
                "required_cpu_features": target["required_cpu_features"],
                "runtime_floor": {"android_api": str(api)},
            },
            "toolchain": {
                "rustc": run(["rustc", "--version", "--verbose"], "rustc"),
                "compiler": run([str(clang), "--version"], "Android clang"),
                "sdk": (
                    f"Android NDK {match.group(1)}; API {api} sysroot; "
                    f"source.properties sha256={hash_file(properties)}"
                ),
                "linker": run([str(linker), "--version"], "Android linker"),
            },
            "runtime": {
                "backend": "v8",
                "rusty_v8_version": version,
                "rusty_v8_revision": source_revision,
                "v8_revision": v8_revision,
                "normalized_gn_args": normalized_gn_arguments(arguments.extra_gn_args, api),
                "patches": patches,
            },
            "hashes": {
                "archive": hash_file(arguments.archive),
                "rust_binding": hash_file(arguments.binding),
            },
            "provenance": {
                "source_revision": source_revision,
                "build_recipe": "scripts/build-v8-android.sh",
                "build_recipe_sha256": hash_file(recipe),
                "licenses": ["BSD-3-Clause", "MIT"],
            },
        }

        tool = arguments.tool
        if tool is None:
            manifest = repo / "tools/artifact-manifest/Cargo.toml"
            subprocess.run(
                ["cargo", "build", "--manifest-path", str(manifest), "--locked", "--release"],
                check=True,
            )
            tool = repo / "tools/artifact-manifest/target/release/migo-artifact-manifest"
        tool = tool.resolve()
        arguments.output.parent.mkdir(parents=True, exist_ok=True)
        descriptor, draft_name = tempfile.mkstemp(
            prefix=".v8-component.", suffix=".json", dir=arguments.output.parent
        )
        os.close(descriptor)
        draft = pathlib.Path(draft_name)
        try:
            write_draft(draft, component)
            subprocess.run(
                [str(tool), "seal-v8-component", str(draft), str(arguments.output)],
                check=True,
            )
            subprocess.run(
                [
                    str(tool),
                    "verify-v8-component",
                    str(arguments.output),
                    str(arguments.archive),
                    str(arguments.binding),
                ],
                check=True,
            )
        finally:
            draft.unlink(missing_ok=True)
        print(f"V8 component manifest -> {arguments.output}")
    except (OSError, RuntimeError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f"V8 component manifest: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
