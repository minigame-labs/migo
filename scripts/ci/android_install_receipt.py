#!/usr/bin/env python3
"""Prove that an installed Android package contains the current AAR native."""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import re
import sys
import zipfile
from pathlib import Path
from typing import Iterable


SCHEMA_VERSION = 1
REVISION_RE = re.compile(r"[0-9a-f]{40}")
SUPPORTED_ABIS = {"arm64-v8a", "x86_64"}


class InstallReceiptError(ValueError):
    pass


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(chunk)
    except OSError as error:
        raise InstallReceiptError(f"could not hash {path.name}: {error}") from error
    return digest.hexdigest()


def _read_unique_member(archive_path: Path, member: str) -> bytes:
    try:
        with zipfile.ZipFile(archive_path) as archive:
            matches = [name for name in archive.namelist() if name == member]
            if len(matches) != 1:
                raise InstallReceiptError(
                    f"{archive_path.name} must contain exactly one {member}"
                )
            return archive.read(member)
    except (OSError, zipfile.BadZipFile, KeyError) as error:
        raise InstallReceiptError(
            f"could not read {member} from {archive_path.name}: {error}"
        ) from error


def create_receipt(
    *,
    revision: str,
    artifact: Path,
    package: str,
    device_abi: str,
    device_serial: str,
    installed_apks: Iterable[Path],
    output: Path,
) -> dict:
    artifact = Path(artifact).resolve()
    output = Path(output)
    installed_apks = [Path(path).resolve() for path in installed_apks]

    if REVISION_RE.fullmatch(revision) is None:
        raise InstallReceiptError("source revision must be a full lowercase Git object ID")
    if not artifact.is_file() or artifact.suffix != ".aar":
        raise InstallReceiptError("artifact must be an existing AAR")
    if device_abi not in SUPPORTED_ABIS:
        raise InstallReceiptError(f"unsupported device ABI: {device_abi}")
    if not package.strip() or not device_serial.strip():
        raise InstallReceiptError("package and device identity are required")
    if not installed_apks or any(not path.is_file() for path in installed_apks):
        raise InstallReceiptError("every installed APK must exist")

    artifact_member = f"jni/{device_abi}/libmigo.so"
    artifact_native = _read_unique_member(artifact, artifact_member)
    artifact_native_sha256 = hashlib.sha256(artifact_native).hexdigest()

    installed_member = f"lib/{device_abi}/libmigo.so"
    matches: list[tuple[Path, bytes]] = []
    for apk in installed_apks:
        try:
            with zipfile.ZipFile(apk) as archive:
                names = [name for name in archive.namelist() if name == installed_member]
                if len(names) > 1:
                    raise InstallReceiptError(
                        f"{apk.name} contains duplicate {installed_member} entries"
                    )
                if names:
                    matches.append((apk, archive.read(installed_member)))
        except (OSError, zipfile.BadZipFile, KeyError) as error:
            raise InstallReceiptError(f"invalid installed APK {apk.name}: {error}") from error

    if len(matches) != 1:
        raise InstallReceiptError(
            f"installed APK set must contain exactly one {installed_member}; found {len(matches)}"
        )
    native_apk, installed_native = matches[0]
    installed_native_sha256 = hashlib.sha256(installed_native).hexdigest()
    if not hmac.compare_digest(installed_native_sha256, artifact_native_sha256):
        raise InstallReceiptError("installed native does not match the current AAR slice")

    document = {
        "schema_version": SCHEMA_VERSION,
        "source_revision": revision,
        "artifact": {
            "filename": artifact.name,
            "sha256": _sha256(artifact),
            "native_member": artifact_member,
            "native_sha256": artifact_native_sha256,
        },
        "installation": {
            "package": package,
            "apks": [
                {"filename": apk.name, "sha256": _sha256(apk)}
                for apk in sorted(installed_apks, key=lambda item: item.name)
            ],
            "native_apk": native_apk.name,
            "native_member": installed_member,
            "installed_native_sha256": installed_native_sha256,
            "verified_against_artifact": True,
        },
        "device": {
            "abi": device_abi,
            "serial_sha256": hashlib.sha256(device_serial.encode("utf-8")).hexdigest(),
        },
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return document


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--artifact", required=True, type=Path)
    parser.add_argument("--package", required=True)
    parser.add_argument("--device-abi", required=True)
    parser.add_argument("--device-serial", required=True)
    parser.add_argument("--installed-apk", action="append", default=[], type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)

    try:
        create_receipt(
            revision=args.revision,
            artifact=args.artifact,
            package=args.package,
            device_abi=args.device_abi,
            device_serial=args.device_serial,
            installed_apks=args.installed_apk,
            output=args.out,
        )
    except InstallReceiptError as error:
        print(f"Android installation evidence: FAIL ({error})", file=sys.stderr)
        return 1
    print(f"Android installation evidence: PASS ({args.out})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
