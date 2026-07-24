#!/usr/bin/env python3
"""Emit and verify the Linux GNU V8 component manifest after a source build."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import shlex
import stat
import subprocess
import tempfile


def run(command: list[str], label: str) -> str:
    try:
        result = subprocess.run(command, check=False, text=True, capture_output=True)
    except OSError as error:
        raise RuntimeError(f"cannot run {label}: {error}") from error
    if result.returncode != 0:
        raise RuntimeError(f"{label} failed: {result.stderr.strip()}")
    output = " | ".join(line.strip() for line in result.stdout.splitlines() if line.strip())
    if not output:
        raise RuntimeError(f"{label} returned empty output")
    return output


def hash_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git_revision(path: pathlib.Path, label: str) -> str:
    revision = run(["git", "-C", str(path), "rev-parse", "HEAD"], label)
    if not re.fullmatch(r"[0-9a-fA-F]{40}", revision):
        raise RuntimeError(f"{label} is not a full revision: {revision!r}")
    return revision


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


def normalized_gn_arguments(value: str) -> list[str]:
    arguments: list[str] = []
    keys: set[str] = set()
    for argument in shlex.split(value):
        if "=" not in argument or not argument.split("=", 1)[0]:
            raise ValueError(f"GN arg must use key=value syntax: {argument!r}")
        key = argument.split("=", 1)[0]
        if key in keys:
            raise ValueError(f"duplicate GN argument key: {key}")
        keys.add(key)
        arguments.append(argument)
    if not arguments:
        raise ValueError("GN argument set must not be empty")
    return sorted(arguments, key=lambda item: item.encode("utf-8"))


def parse_patch(value: str) -> tuple[str, pathlib.Path]:
    patch_id, separator, path = value.partition("=")
    if not separator or not patch_id or not path:
        raise ValueError(f"patch must use id=path syntax: {value!r}")
    return patch_id, pathlib.Path(path)


def changed_paths(path: pathlib.Path) -> list[tuple[str, str]]:
    result = subprocess.run(
        [
            "git", "-C", str(path), "status", "--porcelain=v1", "-z",
            "--untracked-files=all",
        ],
        check=False,
        text=True,
        capture_output=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"git status failed for {path}: {result.stderr.strip()}")
    records: list[tuple[str, str]] = []
    for record in result.stdout.split("\0"):
        if record:
            records.append((record[:2], record[3:]))
    return records


def patch_paths(path: pathlib.Path) -> set[str]:
    paths: set[str] = set()
    for line in path.read_text(encoding="utf-8").splitlines():
        for prefix in ("--- a/", "+++ b/"):
            if not line.startswith(prefix):
                continue
            relative = line.removeprefix(prefix)
            candidate = pathlib.PurePosixPath(relative)
            if candidate.is_absolute() or ".." in candidate.parts:
                raise RuntimeError(f"patch contains unsafe path {relative!r}: {path}")
            paths.add(relative)
    if not paths:
        raise RuntimeError(f"patch declares no changed paths: {path}")
    return paths


def head_blob(source: pathlib.Path, relative: str) -> tuple[bytes, bool] | None:
    """Return one HEAD regular-file blob and its executable bit."""
    tree = subprocess.run(
        ["git", "-C", str(source), "ls-tree", "-z", "HEAD", "--", relative],
        check=False,
        capture_output=True,
    )
    if tree.returncode != 0:
        raise RuntimeError(
            f"git ls-tree failed for {relative}: "
            f"{tree.stderr.decode(errors='replace').strip()}"
        )
    if not tree.stdout:
        return None
    record = tree.stdout.rstrip(b"\0")
    metadata, separator, recorded_path = record.partition(b"\t")
    if not separator or recorded_path.decode("utf-8") != relative:
        raise RuntimeError(f"cannot parse HEAD identity for {relative!r}")
    mode, object_type, object_id = metadata.decode("ascii").split()
    if object_type != "blob" or mode not in {"100644", "100755"}:
        raise RuntimeError(
            f"declared patch path must be a regular tracked file: {relative} ({mode} {object_type})"
        )
    blob = subprocess.run(
        ["git", "-C", str(source), "cat-file", "blob", object_id],
        check=False,
        capture_output=True,
    )
    if blob.returncode != 0:
        raise RuntimeError(
            f"git cat-file failed for {relative}: "
            f"{blob.stderr.decode(errors='replace').strip()}"
        )
    return blob.stdout, mode == "100755"


def verify_exact_patch_result(
    source: pathlib.Path,
    patch_files: list[pathlib.Path],
    declared_paths: set[str],
) -> None:
    """Rebuild the declared result from HEAD and compare exact file bytes.

    `git apply --reverse --check` proves that each declared patch is present,
    but it deliberately tolerates unrelated edits elsewhere in the same file.
    Reconstructing from HEAD closes that provenance gap without modifying the
    V8 source checkout or its index.
    """
    with tempfile.TemporaryDirectory(prefix="migo-v8-patch-check.") as directory:
        reconstructed = pathlib.Path(directory)
        for relative in sorted(declared_paths, key=lambda value: value.encode("utf-8")):
            base = head_blob(source, relative)
            if base is None:
                raise RuntimeError(
                    f"declared patch path is not a regular file in HEAD: {relative}"
                )
            contents, executable = base
            destination = reconstructed / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(contents)
            destination.chmod(0o755 if executable else 0o644)

        for patch in patch_files:
            applied = subprocess.run(
                ["git", "apply", "--unsafe-paths", "--whitespace=nowarn", str(patch)],
                cwd=reconstructed,
                check=False,
                capture_output=True,
                text=True,
            )
            if applied.returncode != 0:
                raise RuntimeError(
                    f"cannot reconstruct declared patch result from HEAD ({patch}): "
                    f"{applied.stderr.strip()}"
                )

        for relative in sorted(declared_paths, key=lambda value: value.encode("utf-8")):
            expected = reconstructed / relative
            actual = source / relative
            if not expected.is_file() or expected.is_symlink():
                raise RuntimeError(
                    f"declared patch result is not a regular file: {relative}"
                )
            if not actual.is_file() or actual.is_symlink():
                raise RuntimeError(
                    f"rusty_v8 worktree path is not a regular file: {relative}"
                )
            expected_executable = bool(expected.stat().st_mode & stat.S_IXUSR)
            actual_executable = bool(actual.stat().st_mode & stat.S_IXUSR)
            if (
                expected.read_bytes() != actual.read_bytes()
                or expected_executable != actual_executable
            ):
                raise RuntimeError(
                    "declared patches do not exactly reproduce rusty_v8 worktree "
                    f"bytes and mode for {relative}"
                )


def verify_source_changes(
    source: pathlib.Path, patches: list[tuple[str, pathlib.Path]]
) -> list[dict]:
    allowed: set[str] = set()
    identities: list[dict] = []
    patch_files: list[pathlib.Path] = []
    for patch_id, path in patches:
        path = path.resolve()
        if not path.is_file():
            raise RuntimeError(f"missing declared source patch: {path}")
        allowed.update(patch_paths(path))
        patch_files.append(path)
        applied = subprocess.run(
            ["git", "-C", str(source), "apply", "--reverse", "--check", str(path)],
            check=False,
            capture_output=True,
            text=True,
        )
        if applied.returncode != 0:
            raise RuntimeError(f"declared patch is not applied: {patch_id} ({path})")
        identities.append({"id": patch_id, "sha256": hash_file(path)})

    # `build` is a nested git submodule. When its working tree carries changes,
    # git reports the parent's view of it as a dirty pointer (status " m build"
    # for an in-tree change, " M build" for a moved pointer) -- which no `--patch`
    # ever declares, because patches touch paths *inside* the submodule. The
    # Linux build declares no build-submodule patches: it uses the bullseye
    # sysroot and its own GN args, not the Android sysroot/libcxx patches. So the
    # correct invariant is that the `build` submodule is pristine, and the check
    # is separated out to say so instead of surfacing a cryptic " m build".
    build_submodule = source / "build"
    if (build_submodule / ".git").exists():
        build_changes = changed_paths(build_submodule)
        if build_changes:
            raise RuntimeError(
                "the `build` submodule has working-tree changes, but a Linux V8 "
                "build declares no build-submodule patches (it uses the bullseye "
                "sysroot, not the Android build patches). Reset it before "
                f"building: {[f'{s} {p}' for s, p in build_changes]}"
            )

    # The parent's dirty pointer for a pristine `build` submodule (git can still
    # report " m build" transiently) is expected and allowed alongside the
    # declared top-level patches; nothing else is.
    allowed_top = allowed | {"build"}
    unexpected = [
        f"{status} {path}"
        for status, path in changed_paths(source)
        if path not in allowed_top or status not in {" M", "M ", "MM", " m", "m "}
    ]
    if unexpected:
        raise RuntimeError(
            f"rusty_v8 source has changes not represented by --patch: {unexpected}"
        )
    verify_exact_patch_result(source, patch_files, allowed)
    if changed_paths(source / "v8"):
        raise RuntimeError("V8 source has unrecorded tracked or untracked changes")
    identities.sort(key=lambda item: item["id"].encode("utf-8"))
    return identities


def build_component(
    *,
    rusty_v8_version: str,
    rusty_v8_revision: str,
    v8_revision: str,
    gn_args: list[str],
    patches: list[dict],
    archive_sha256: str,
    binding_sha256: str,
    rustc: str,
    compiler: str,
    sdk: str,
    linker: str,
    recipe_sha256: str,
) -> dict:
    return {
        "schema": "migo-v8-component-manifest/v1",
        "component_id": "",
        "target": {
            "triple": "x86_64-unknown-linux-gnu",
            "os": "linux",
            "arch": "x86_64",
            "abi": "gnu",
            "cpu_baseline": "x86-64-v1",
            "required_cpu_features": ["cmov", "sse2"],
            "runtime_floor": {"glibc": "2.31", "glibcxx": "3.4.28"},
        },
        "toolchain": {
            "rustc": rustc,
            "compiler": compiler,
            "sdk": sdk,
            "linker": linker,
        },
        "runtime": {
            "backend": "v8",
            "rusty_v8_version": rusty_v8_version,
            "rusty_v8_revision": rusty_v8_revision,
            "v8_revision": v8_revision,
            "normalized_gn_args": gn_args,
            "patches": patches,
        },
        "hashes": {
            "archive": archive_sha256,
            "rust_binding": binding_sha256,
        },
        "provenance": {
            "source_revision": rusty_v8_revision,
            "build_recipe": "scripts/build-v8-linux.sh",
            "build_recipe_sha256": recipe_sha256,
            "licenses": ["BSD-3-Clause", "MIT"],
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", required=True, type=pathlib.Path)
    parser.add_argument("--rusty-v8-src", required=True, type=pathlib.Path)
    parser.add_argument("--gn-args", required=True)
    parser.add_argument("--archive", required=True, type=pathlib.Path)
    parser.add_argument("--binding", required=True, type=pathlib.Path)
    parser.add_argument("--sysroot", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--patch", action="append", default=[])
    parser.add_argument("--compiler", default=os.environ.get("CXX", "clang++"))
    parser.add_argument("--linker", default=os.environ.get("MIGO_LLD", "ld.lld"))
    parser.add_argument("--tool", type=pathlib.Path)
    arguments = parser.parse_args()

    repo = arguments.repo_root.resolve()
    source = arguments.rusty_v8_src.resolve()
    archive = arguments.archive.resolve()
    binding = arguments.binding.resolve()
    sysroot = arguments.sysroot.resolve()
    for label, path in [("archive", archive), ("binding", binding)]:
        if not path.is_file():
            raise RuntimeError(f"missing V8 {label}: {path}")
    if not sysroot.is_dir():
        raise RuntimeError(f"missing Linux sysroot: {sysroot}")

    patches = verify_source_changes(source, [parse_patch(value) for value in arguments.patch])
    sysroot_recipe = source / "build/linux/sysroot_scripts/sysroots.json"
    if not sysroot_recipe.is_file():
        raise RuntimeError(f"missing Chromium sysroot identity: {sysroot_recipe}")
    sdk = (
        "Debian bullseye amd64 sysroot; "
        f"sysroots.json sha256={hash_file(sysroot_recipe)}"
    )
    component = build_component(
        rusty_v8_version=package_version(source / "Cargo.toml"),
        rusty_v8_revision=git_revision(source, "rusty_v8 revision"),
        v8_revision=git_revision(source / "v8", "V8 revision"),
        gn_args=normalized_gn_arguments(arguments.gn_args),
        patches=patches,
        archive_sha256=hash_file(archive),
        binding_sha256=hash_file(binding),
        rustc=run(["rustc", "--version", "--verbose"], "rustc"),
        compiler=run([arguments.compiler, "--version"], "Linux C++ compiler"),
        sdk=sdk,
        linker=run([arguments.linker, "--version"], "Linux linker"),
        recipe_sha256=hash_file(repo / "scripts/build-v8-linux.sh"),
    )

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
        prefix=".linux-v8-component.", suffix=".json", dir=arguments.output.parent
    )
    os.close(descriptor)
    draft = pathlib.Path(draft_name)
    try:
        draft.write_text(json.dumps(component, indent=2, sort_keys=True) + "\n")
        subprocess.run(
            [str(tool), "seal-v8-component", str(draft), str(arguments.output)],
            check=True,
        )
        subprocess.run(
            [str(tool), "verify-v8-component", str(arguments.output), str(archive), str(binding)],
            check=True,
        )
    finally:
        draft.unlink(missing_ok=True)
    print(f"verified Linux V8 component manifest -> {arguments.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
