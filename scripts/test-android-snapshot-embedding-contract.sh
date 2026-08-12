#!/usr/bin/env bash
# Every snapshot an AAR's slice manifest claims must actually be in that slice's
# libmigo.so.
#
# The two existing checks are both about the snapshot *file*, not the shipped bytes:
#
#   * scripts/check-snapshot-freshness.sh proves the committed .bin matches the
#     inputs that produced it.
#   * generate-android-artifact-manifests.py validates the .bin again and refuses to
#     emit a slice manifest without it, and test-android-release-manifest-gate.sh
#     proves a release AAR cannot skip manifest generation.
#
# Neither observes the .so. runtime-v8/build.rs makes an independent embedding
# decision and fails *safe*: every failure path prints `cargo:warning=...; loading JS
# from source` and continues. So the slice can record a snapshot the binary does not
# contain, and the result is a cold-start regression with nothing red -- the artifact
# still works, it just parses extension JS at startup instead of deserialising a heap.
#
# Asserting on the artifact rather than inside build.rs is deliberate, and not only
# for coverage. runtime-v8's snapshot fingerprint walks every .rs file in the crate
# directory (build_snapshot.rs collect_rust_files -> collect_files_with_extension),
# which includes build.rs itself. Editing build.rs therefore invalidates all six
# committed snapshots, and regenerating the aarch64 ones needs physical arm64 hardware
# (scripts/gen-snapshot.sh: hosted CI has no arm64 KVM). A validator that cannot be
# changed without hardware is a validator that will not be changed. This gate lives
# outside the fingerprint, and it proves the stronger property: not that the build
# script intended to embed, but that the bytes are there.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ $# -eq 0 ]]; then
    echo "usage: $0 <aar> [<aar>...]" >&2
    echo "" >&2
    echo "Checks every slice of every AAR given. Pass the AARs a release publishes," >&2
    echo "not a glob that can match zero files -- a gate over nothing passes." >&2
    exit 2
fi

python3 - "$ROOT_DIR" "$@" <<'PY'
from __future__ import annotations

import hashlib
import json
import pathlib
import sys
import zipfile

INDEX_PATH = "assets/migo/artifacts/package-index.json"


class ContractError(Exception):
    pass


def sha256_bytes(payload: bytes) -> str:
    digest = hashlib.sha256()
    digest.update(payload)
    return digest.hexdigest()


def snapshot_filename(entry: dict[str, object], where: str) -> str:
    """Mirror the name runtime-v8/build.rs builds for the same record.

    This checks Android AARs specifically (jni/<abi>/libmigo.so below), so the OS
    segment is always "android". build.rs uses SNAPSHOT-{profile}-android-{arch}.bin
    for the host heap and SNAPSHOT-worker-{profile}-android-{arch}.bin for the worker
    heap. Deriving it here from the slice's own fields, rather than accepting a path
    out of the manifest, is what makes the two agree by construction.
    """
    kind = entry.get("runtime_kind")
    profile = entry.get("product_profile")
    arch = entry.get("arch")
    for field, value in (("runtime_kind", kind), ("product_profile", profile), ("arch", arch)):
        if not isinstance(value, str) or not value:
            raise ContractError(f"{where}: snapshot record has no usable {field}")
    if kind == "host":
        return f"SNAPSHOT-{profile}-android-{arch}.bin"
    if kind == "worker":
        return f"SNAPSHOT-worker-{profile}-android-{arch}.bin"
    raise ContractError(f"{where}: unknown snapshot runtime_kind {kind!r}")


def check_slice(
    archive: zipfile.ZipFile,
    snapshots_dir: pathlib.Path,
    aar_name: str,
    manifest_path: str,
) -> list[str]:
    abi = pathlib.PurePosixPath(manifest_path).stem
    where = f"{aar_name} slice {abi}"

    manifest = json.loads(archive.read(manifest_path))
    if not isinstance(manifest, dict):
        raise ContractError(f"{where}: slice manifest is not an object")
    records = manifest.get("snapshots")
    if not isinstance(records, list) or not records:
        raise ContractError(
            f"{where}: slice manifest declares no snapshots. A slice with an empty "
            "snapshots array means the artifact ships without an embedded V8 heap"
        )

    library_entry = f"jni/{abi}/libmigo.so"
    try:
        library = archive.read(library_entry)
    except KeyError as error:
        raise ContractError(f"{where}: {library_entry} is not in the AAR") from error

    results: list[str] = []
    for record in records:
        if not isinstance(record, dict):
            raise ContractError(f"{where}: snapshot record is not an object")
        name = snapshot_filename(record, where)
        declared = record.get("bytes_hash")
        if not isinstance(declared, str) or len(declared) != 64:
            raise ContractError(f"{where}: {name} has no sha256 bytes_hash")

        blob_path = snapshots_dir / name
        if not blob_path.is_file():
            raise ContractError(
                f"{where}: the slice claims {name} but that file is not in "
                f"{snapshots_dir}. The AAR was built against a snapshot this tree no "
                "longer has, so what it embedded cannot be checked"
            )
        blob = blob_path.read_bytes()
        actual = sha256_bytes(blob)
        if actual != declared:
            raise ContractError(
                f"{where}: {name} in this tree hashes to {actual} but the slice claims "
                f"{declared}. Either the AAR is stale or the snapshot was regenerated "
                "after it was built; rebuild the AAR"
            )

        if library.find(blob) < 0:
            raise ContractError(
                f"{where}: {library_entry} does not contain the {len(blob)} bytes of "
                f"{name}, which the slice manifest claims it embeds. build.rs fails "
                "safe and only warns, so check its output for "
                "'loading JS from source' -- the artifact runs, it just parses "
                "extension JS at startup instead of deserialising a snapshot"
            )
        results.append(f"{where}: {library_entry} embeds {name} ({len(blob)} bytes)")
    return results


def main() -> int:
    root = pathlib.Path(sys.argv[1]).resolve()
    snapshots_dir = root / "engine/crates/runtime-v8/snapshots"
    failures: list[str] = []
    passes: list[str] = []

    for raw in sys.argv[2:]:
        aar = pathlib.Path(raw)
        if not aar.is_file():
            failures.append(f"{raw}: not a file")
            continue
        try:
            with zipfile.ZipFile(aar) as archive:
                index = json.loads(archive.read(INDEX_PATH))
                slices = index.get("slices") if isinstance(index, dict) else None
                if not isinstance(slices, list) or not slices:
                    raise ContractError(f"{aar.name}: package index has no slices")
                for entry in slices:
                    if not isinstance(entry, dict):
                        raise ContractError(f"{aar.name}: package index slice is not an object")
                    manifest_path = entry.get("manifest_path")
                    if not isinstance(manifest_path, str) or not manifest_path:
                        raise ContractError(f"{aar.name}: package index slice has no manifest_path")
                    passes.extend(check_slice(archive, snapshots_dir, aar.name, manifest_path))
        except (ContractError, KeyError, ValueError, zipfile.BadZipFile) as error:
            failures.append(str(error))

    for line in passes:
        print(f"OK: {line}")
    for line in failures:
        print(f"Android snapshot embedding contract: {line}", file=sys.stderr)

    if failures:
        return 1
    if not passes:
        print(
            "Android snapshot embedding contract checked nothing: no slice in any AAR "
            "given declared a snapshot. This is a failure, not a pass.",
            file=sys.stderr,
        )
        return 1
    print(f"Android snapshot embedding contract: PASS ({len(passes)} slice snapshot(s) verified)")
    return 0


sys.exit(main())
PY
