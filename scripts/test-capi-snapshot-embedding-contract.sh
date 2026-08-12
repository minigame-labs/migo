#!/usr/bin/env bash
# Every C-ABI package that claims an embedded V8 snapshot must actually carry
# those bytes in its shipped library, not just in its manifest.
#
# scripts/test-android-snapshot-embedding-contract.sh proves this for the AAR's
# jni/<abi>/libmigo.so, but the same property was unchecked for the C-ABI
# packages (Android's libmigo_capi.a and Linux's libmigo.a / libmigo.so.<ver>):
# generate-android-artifact-manifests.py / gen-linux-package-metadata.py
# validate the snapshot *file* and the manifest's declared bytes_hash, but
# nothing reads the compiled library to confirm the bytes actually landed
# there. runtime-v8/build.rs fails *safe* on every rejection path (warns and
# falls back to source JS), so a package can claim embedding it does not ship.
#
# Unlike the AAR contract, a package may legitimately declare
# snapshot_policy=none (Linux before this change, OpenHarmony and Windows
# until their own V8 archives exist) -- that is a checked, honest fact, not a
# gap. Only "embedded" packages are held to the byte-containment proof.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ $# -eq 0 ]]; then
    echo "usage: $0 <package-root> [<package-root>...]" >&2
    echo "" >&2
    echo "package-root is a staged SDK prefix (e.g. dist/migo-android-arm64," >&2
    echo "dist/migo-linux-x86_64) containing share/migo/*-manifest.json." >&2
    exit 2
fi

python3 - "$ROOT_DIR" "$@" <<'PY'
from __future__ import annotations

import hashlib
import json
import pathlib
import sys


class ContractError(Exception):
    pass


def sha256_bytes(payload: bytes) -> str:
    digest = hashlib.sha256()
    digest.update(payload)
    return digest.hexdigest()


def check_package(root: pathlib.Path, snapshots_dir: pathlib.Path, package_root: pathlib.Path) -> list[str]:
    where_prefix = package_root.name
    manifests = sorted((package_root / "share/migo").glob("*-manifest.json"))
    if not manifests:
        raise ContractError(f"{where_prefix}: no share/migo/*-manifest.json in the staged package")
    if len(manifests) > 1:
        raise ContractError(
            f"{where_prefix}: {len(manifests)} package manifests found, expected exactly one: "
            f"{[m.name for m in manifests]}"
        )
    manifest_path = manifests[0]
    manifest = json.loads(manifest_path.read_text())
    where = f"{where_prefix}/{manifest_path.name}"

    policy = manifest.get("snapshot_policy")
    records = manifest.get("snapshots")
    if not isinstance(records, list):
        raise ContractError(f"{where}: manifest has no snapshots list")

    if policy == "none":
        if records:
            raise ContractError(f"{where}: snapshot_policy is none but snapshots were listed")
        return [f"{where}: snapshot_policy=none, nothing to embed (skipped)"]
    if policy != "embedded":
        raise ContractError(f"{where}: unknown snapshot_policy {policy!r}")
    if not records:
        raise ContractError(f"{where}: snapshot_policy is embedded but no snapshot was listed")

    os_word = manifest.get("os")
    if not isinstance(os_word, str) or not os_word:
        raise ContractError(f"{where}: manifest has no usable os field")

    artifacts = manifest.get("artifacts")
    if not isinstance(artifacts, dict) or not artifacts:
        raise ContractError(f"{where}: manifest has no artifacts list")
    candidates = [
        package_root / relative
        for relative in artifacts
        if relative.endswith(".a") or ".so" in relative.split("/")[-1]
    ]
    candidates = [path for path in candidates if path.is_file() and not path.is_symlink()]
    if not candidates:
        raise ContractError(f"{where}: no non-symlink .a/.so artifact to check for embedded bytes")
    libraries = {path: path.read_bytes() for path in candidates}

    results: list[str] = []
    for record in records:
        if not isinstance(record, dict):
            raise ContractError(f"{where}: snapshot record is not an object")
        name = snapshot_filename_for(record, where, os_word)
        declared = record.get("bytes_hash")
        if not isinstance(declared, str) or len(declared) != 64:
            raise ContractError(f"{where}: {name} has no sha256 bytes_hash")

        blob_path = snapshots_dir / name
        if not blob_path.is_file():
            raise ContractError(
                f"{where}: the package claims {name} but that file is not in "
                f"{snapshots_dir}. The package was built against a snapshot this tree "
                "no longer has, so what it embedded cannot be checked"
            )
        blob = blob_path.read_bytes()
        actual = sha256_bytes(blob)
        if actual != declared:
            raise ContractError(
                f"{where}: {name} in this tree hashes to {actual} but the package claims "
                f"{declared}. Either the package is stale or the snapshot was regenerated "
                "after it was built; rebuild the package"
            )

        found_in = [path for path, data in libraries.items() if data.find(blob) >= 0]
        if not found_in:
            raise ContractError(
                f"{where}: none of {[p.relative_to(package_root).as_posix() for p in candidates]} "
                f"contain the {len(blob)} bytes of {name}, which the package manifest claims it "
                "embeds. build.rs fails safe and only warns, so check its output for "
                "'loading JS from source' -- the artifact runs, it just parses extension JS at "
                "startup instead of deserialising a snapshot"
            )
        results.append(
            f"{where}: {[p.relative_to(package_root).as_posix() for p in found_in]} embeds "
            f"{name} ({len(blob)} bytes)"
        )
    return results


def snapshot_filename_for(record: dict[str, object], where: str, os_word: str) -> str:
    kind = record.get("runtime_kind")
    profile = record.get("product_profile")
    arch = record.get("arch")
    for field, value in (("runtime_kind", kind), ("product_profile", profile), ("arch", arch)):
        if not isinstance(value, str) or not value:
            raise ContractError(f"{where}: snapshot record has no usable {field}")
    if kind == "host":
        return f"SNAPSHOT-{profile}-{os_word}-{arch}.bin"
    if kind == "worker":
        return f"SNAPSHOT-worker-{profile}-{os_word}-{arch}.bin"
    raise ContractError(f"{where}: unknown snapshot runtime_kind {kind!r}")


def main() -> int:
    root = pathlib.Path(sys.argv[1]).resolve()
    snapshots_dir = root / "engine/crates/runtime-v8/snapshots"
    failures: list[str] = []
    passes: list[str] = []

    for raw in sys.argv[2:]:
        package_root = pathlib.Path(raw).resolve()
        if not package_root.is_dir():
            failures.append(f"{raw}: not a directory")
            continue
        try:
            passes.extend(check_package(root, snapshots_dir, package_root))
        except (ContractError, KeyError, ValueError, OSError) as error:
            failures.append(str(error))

    for line in passes:
        print(f"OK: {line}")
    for line in failures:
        print(f"C-ABI snapshot embedding contract: {line}", file=sys.stderr)

    if failures:
        return 1
    if not passes:
        print(
            "C-ABI snapshot embedding contract checked nothing: no package given had a manifest.",
            file=sys.stderr,
        )
        return 1
    print(f"C-ABI snapshot embedding contract: PASS ({len(passes)} package(s) checked)")
    return 0


sys.exit(main())
PY
