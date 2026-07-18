#!/usr/bin/env python3
"""Verify that an AAR embeds exactly the staged Migo package index and slices."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile
import zipfile


INDEX_PATH = "assets/migo/artifacts/package-index.json"
ANDROID_ABIS = {
    ("aarch64", "aarch64-linux-android"): "arm64-v8a",
    ("x86_64", "x86_64-linux-android"): "x86_64",
}


def safe_package_path(value: object) -> pathlib.PurePosixPath:
    if not isinstance(value, str) or not value or "\\" in value:
        raise RuntimeError("manifest_path must be a non-empty '/'-separated path")
    path = pathlib.PurePosixPath(value)
    if path.is_absolute() or any(part in ("", ".", "..") for part in path.parts):
        raise RuntimeError(f"unsafe manifest_path in package index: {value!r}")
    return path


def zip_entry_sha256(archive: zipfile.ZipFile, name: str) -> str:
    digest = hashlib.sha256()
    with archive.open(name) as source:
        for chunk in iter(lambda: source.read(64 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--aar", required=True, type=pathlib.Path)
    parser.add_argument("--index", required=True, type=pathlib.Path)
    parser.add_argument("--tool", required=True, type=pathlib.Path)
    arguments = parser.parse_args()

    try:
        index_bytes = arguments.index.read_bytes()
        index = json.loads(index_bytes)
        slices = index.get("slices") if isinstance(index, dict) else None
        if not isinstance(slices, list) or not slices:
            raise RuntimeError("package index has no slices")
        with zipfile.ZipFile(arguments.aar) as archive:
            names = [entry.filename for entry in archive.infolist()]
            if names.count(INDEX_PATH) != 1:
                raise RuntimeError(f"AAR must contain exactly one {INDEX_PATH}")
            if archive.read(INDEX_PATH) != index_bytes:
                raise RuntimeError("embedded package index differs from the staged index")
            with tempfile.TemporaryDirectory(prefix="migo-aar-manifest-") as directory:
                root = pathlib.Path(directory)
                expected_slice_entries: set[str] = set()
                expected_jni_entries: set[str] = set()
                for entry in slices:
                    if not isinstance(entry, dict):
                        raise RuntimeError("package index slice entry must be an object")
                    path = safe_package_path(entry.get("manifest_path"))
                    name = path.as_posix()
                    expected_slice_entries.add(name)
                    if names.count(name) != 1:
                        raise RuntimeError(f"AAR must contain exactly one {name}")
                    slice_bytes = archive.read(name)
                    slice_manifest = json.loads(slice_bytes)
                    if not isinstance(slice_manifest, dict):
                        raise RuntimeError(f"slice manifest must be an object: {name}")
                    target = slice_manifest.get("target")
                    hashes = slice_manifest.get("hashes")
                    if not isinstance(target, dict) or not isinstance(hashes, dict):
                        raise RuntimeError(f"slice target/hashes are missing: {name}")
                    target_key = (target.get("arch"), target.get("triple"))
                    abi = ANDROID_ABIS.get(target_key)
                    if abi is None:
                        raise RuntimeError(f"unsupported Android target in slice: {target_key!r}")
                    for library, hash_field in (
                        ("libmigo.so", "runtime_binary"),
                        ("libc++_shared.so", "cxx_runtime"),
                    ):
                        library_path = f"jni/{abi}/{library}"
                        expected_jni_entries.add(library_path)
                        if names.count(library_path) != 1:
                            raise RuntimeError(
                                f"AAR must contain exactly one {library_path}"
                            )
                        declared_hash = hashes.get(hash_field)
                        actual_hash = zip_entry_sha256(archive, library_path)
                        if declared_hash != actual_hash:
                            raise RuntimeError(
                                f"{library_path} hash mismatch "
                                f"(slice={declared_hash}, package={actual_hash})"
                            )
                    destination = root.joinpath(*path.parts)
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    destination.write_bytes(slice_bytes)
                actual_slice_entries = {
                    name
                    for name in names
                    if name.startswith("assets/migo/artifacts/slices/")
                    and name.endswith(".json")
                }
                if actual_slice_entries != expected_slice_entries:
                    extras = sorted(actual_slice_entries - expected_slice_entries)
                    missing = sorted(expected_slice_entries - actual_slice_entries)
                    raise RuntimeError(
                        "AAR slice entries differ from package index "
                        f"(missing={missing}, unindexed={extras})"
                    )
                actual_jni_entries = {
                    name
                    for name in names
                    if name.startswith("jni/")
                    and pathlib.PurePosixPath(name).name
                    in {"libmigo.so", "libc++_shared.so"}
                }
                unindexed_jni = sorted(actual_jni_entries - expected_jni_entries)
                if unindexed_jni:
                    raise RuntimeError(
                        f"AAR has unindexed Migo JNI entries: {unindexed_jni}"
                    )
                result = subprocess.run(
                    [str(arguments.tool), "verify-index", str(arguments.index), str(root)],
                    check=False,
                    text=True,
                    capture_output=True,
                )
                if result.returncode != 0:
                    raise RuntimeError(result.stderr.strip() or "package index verification failed")
        print(f"verified embedded Android artifact manifests: {arguments.aar}")
    except (OSError, RuntimeError, ValueError, zipfile.BadZipFile, json.JSONDecodeError) as error:
        print(f"AAR manifest verification: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
