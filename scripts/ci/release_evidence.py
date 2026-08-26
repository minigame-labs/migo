#!/usr/bin/env python3
"""Create and verify artifact-bound Android release evidence."""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import re
import sys
import zipfile
from pathlib import Path
from typing import Mapping


SCHEMA_VERSION = 1
EXPECTED_REPORTS = (
    "perf_metrics",
    "perf_summary",
    "power_metrics",
    "power_summary",
    "render_summary",
    "suite_summary",
)
SUMMARY_STATUS = {
    "perf_summary": ("overall", "pass"),
    "power_summary": ("overall", "pass"),
    "render_summary": ("pass", True),
    "suite_summary": ("overall", "pass"),
}
DEVICE_BOUND_REPORTS = {
    "perf_metrics",
    "perf_summary",
    "power_metrics",
    "power_summary",
    "suite_summary",
}
REVISION_RE = re.compile(r"[0-9a-f]{40}")
SHA256_RE = re.compile(r"[0-9a-f]{64}")
SUPPORTED_ABIS = {"arm64-v8a", "x86_64"}


class EvidenceError(ValueError):
    pass


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _json(path: Path) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EvidenceError(f"invalid JSON report {path.name}: {error}") from error
    if not isinstance(value, dict):
        raise EvidenceError(f"report {path.name} must contain a JSON object")
    return value


def _native_member(abi: str) -> str:
    if abi not in SUPPORTED_ABIS:
        raise EvidenceError(f"unsupported device ABI: {abi}")
    return f"jni/{abi}/libmigo.so"


def _native_hash(artifact: Path, abi: str) -> tuple[str, str]:
    member = _native_member(abi)
    try:
        with zipfile.ZipFile(artifact) as archive:
            payload = archive.read(member)
    except (OSError, zipfile.BadZipFile, KeyError) as error:
        raise EvidenceError(f"artifact does not contain {member}: {error}") from error
    return member, hashlib.sha256(payload).hexdigest()


def _report_bindings(
    *,
    revision: str,
    artifact_sha256: str,
    profile: str,
    installed_native_sha256: str,
    device_abi: str,
    package: str,
) -> tuple[dict[str, object], dict[str, object]]:
    common = {
        "_source_revision": revision,
        "_artifact_sha256": artifact_sha256,
        "_profile": profile,
    }
    device = {
        **common,
        "_installed_native_sha256": installed_native_sha256,
        "_device_abi": device_abi,
        "_package": package,
    }
    return common, device


def _validate_report_set(
    reports: Mapping[str, Path],
    *,
    common_binding: Mapping[str, object],
    device_binding: Mapping[str, object],
) -> dict[str, dict]:
    if set(reports) != set(EXPECTED_REPORTS):
        missing = sorted(set(EXPECTED_REPORTS) - set(reports))
        extra = sorted(set(reports) - set(EXPECTED_REPORTS))
        raise EvidenceError(f"report set mismatch (missing={missing}, extra={extra})")

    names = [path.name for path in reports.values()]
    if len(set(names)) != len(names):
        raise EvidenceError("report filenames must be unique")

    documents = {kind: _json(Path(path)) for kind, path in reports.items()}
    for kind, document in documents.items():
        expected = device_binding if kind in DEVICE_BOUND_REPORTS else common_binding
        for field, value in expected.items():
            if document.get(field) != value:
                raise EvidenceError(f"{kind} binding mismatch: {field}")
    for kind in ("perf_metrics", "power_metrics"):
        samples = documents[kind].get("_samples")
        if not isinstance(samples, int) or isinstance(samples, bool) or samples <= 0:
            raise EvidenceError(f"{kind} records no device samples")
    for kind, (field, expected) in SUMMARY_STATUS.items():
        if documents[kind].get(field) != expected:
            raise EvidenceError(f"{kind} did not pass its release gate")
    return documents


def create_evidence(
    *,
    revision: str,
    artifact: Path,
    profile: str,
    device_abi: str,
    installed_native_sha256: str,
    package: str,
    device_model: str,
    android_api: int,
    device_serial: str,
    reports: Mapping[str, Path],
    output: Path,
) -> dict:
    artifact = Path(artifact).resolve()
    output = Path(output)
    reports = {kind: Path(path).resolve() for kind, path in reports.items()}

    if REVISION_RE.fullmatch(revision) is None:
        raise EvidenceError("source revision must be a full lowercase Git object ID")
    if not artifact.is_file() or artifact.suffix != ".aar":
        raise EvidenceError("Android release artifact must be an existing AAR")
    if profile != "full":
        raise EvidenceError("release device evidence must exercise the full product profile")
    if not package.strip():
        raise EvidenceError("installed package name is empty")
    if not device_model.strip() or not device_serial.strip():
        raise EvidenceError("device identity is incomplete")
    if not isinstance(android_api, int) or isinstance(android_api, bool) or android_api < 26:
        raise EvidenceError("Android API level is below the supported floor")
    if SHA256_RE.fullmatch(installed_native_sha256) is None:
        raise EvidenceError("installed native SHA-256 is malformed")

    member, artifact_native_sha256 = _native_hash(artifact, device_abi)
    if not hmac.compare_digest(installed_native_sha256, artifact_native_sha256):
        raise EvidenceError("installed native does not match the release artifact slice")
    artifact_sha256 = _sha256(artifact)
    common_binding, device_binding = _report_bindings(
        revision=revision,
        artifact_sha256=artifact_sha256,
        profile=profile,
        installed_native_sha256=installed_native_sha256,
        device_abi=device_abi,
        package=package,
    )
    _validate_report_set(
        reports,
        common_binding=common_binding,
        device_binding=device_binding,
    )

    document = {
        "schema_version": SCHEMA_VERSION,
        "source_revision": revision,
        "artifact": {
            "filename": artifact.name,
            "kind": "android-aar",
            "profile": profile,
            "sha256": artifact_sha256,
            "native": {
                "abi": device_abi,
                "member": member,
                "sha256": artifact_native_sha256,
            },
        },
        "installation": {
            "package": package,
            "installed_native_sha256": installed_native_sha256,
            "verified_against_artifact": True,
        },
        "device": {
            "abi": device_abi,
            "android_api": android_api,
            "model": device_model,
            "serial_sha256": hashlib.sha256(device_serial.encode("utf-8")).hexdigest(),
        },
        "reports": {
            kind: {"filename": path.name, "sha256": _sha256(path)}
            for kind, path in sorted(reports.items())
        },
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return document


def verify_evidence(
    *, evidence: Path, revision: str, artifact: Path, reports_dir: Path
) -> dict:
    evidence = Path(evidence).resolve()
    artifact = Path(artifact).resolve()
    reports_dir = Path(reports_dir).resolve()
    document = _json(evidence)

    if document.get("schema_version") != SCHEMA_VERSION:
        raise EvidenceError("unsupported evidence schema version")
    if document.get("source_revision") != revision:
        raise EvidenceError("evidence revision does not match the release revision")

    artifact_record = document.get("artifact")
    if not isinstance(artifact_record, dict):
        raise EvidenceError("evidence artifact record is missing")
    if artifact_record.get("filename") != artifact.name:
        raise EvidenceError("evidence names a different release artifact")
    if artifact_record.get("profile") != "full":
        raise EvidenceError("evidence did not exercise the full product profile")
    if artifact_record.get("sha256") != _sha256(artifact):
        raise EvidenceError("release artifact hash does not match device evidence")

    native_record = artifact_record.get("native")
    if not isinstance(native_record, dict):
        raise EvidenceError("native artifact binding is missing")
    member, native_sha256 = _native_hash(artifact, native_record.get("abi", ""))
    if native_record.get("member") != member or native_record.get("sha256") != native_sha256:
        raise EvidenceError("native artifact binding does not match the AAR")

    installation = document.get("installation")
    if not isinstance(installation, dict):
        raise EvidenceError("installation binding is missing")
    if installation.get("verified_against_artifact") is not True:
        raise EvidenceError("installation was not verified against the artifact")
    if not hmac.compare_digest(
        str(installation.get("installed_native_sha256", "")), native_sha256
    ):
        raise EvidenceError("installed native does not match the release artifact slice")

    report_records = document.get("reports")
    if not isinstance(report_records, dict) or set(report_records) != set(EXPECTED_REPORTS):
        raise EvidenceError("evidence report set is incomplete")
    report_paths: dict[str, Path] = {}
    for kind, record in report_records.items():
        if not isinstance(record, dict):
            raise EvidenceError(f"invalid report record: {kind}")
        filename = record.get("filename")
        if not isinstance(filename, str) or Path(filename).name != filename:
            raise EvidenceError(f"unsafe report filename: {kind}")
        path = reports_dir / filename
        if not path.is_file():
            raise EvidenceError(f"missing evidence report: {filename}")
        if record.get("sha256") != _sha256(path):
            raise EvidenceError(f"report hash mismatch: {kind}")
        report_paths[kind] = path
    common_binding, device_binding = _report_bindings(
        revision=revision,
        artifact_sha256=str(artifact_record.get("sha256", "")),
        profile=str(artifact_record.get("profile", "")),
        installed_native_sha256=str(
            installation.get("installed_native_sha256", "")
        ),
        device_abi=str(native_record.get("abi", "")),
        package=str(installation.get("package", "")),
    )
    _validate_report_set(
        report_paths,
        common_binding=common_binding,
        device_binding=device_binding,
    )
    return document


def _report_args(values: list[str]) -> dict[str, Path]:
    reports = {}
    for value in values:
        if "=" not in value:
            raise EvidenceError(f"report must use KIND=PATH: {value}")
        kind, path = value.split("=", 1)
        if kind in reports:
            raise EvidenceError(f"duplicate report kind: {kind}")
        reports[kind] = Path(path)
    return reports


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    create = subparsers.add_parser("create")
    create.add_argument("--revision", required=True)
    create.add_argument("--artifact", required=True, type=Path)
    create.add_argument("--profile", required=True)
    create.add_argument("--device-abi", required=True)
    create.add_argument("--installed-native-sha256", required=True)
    create.add_argument("--package", required=True)
    create.add_argument("--device-model", required=True)
    create.add_argument("--android-api", required=True, type=int)
    create.add_argument("--device-serial", required=True)
    create.add_argument("--report", action="append", default=[])
    create.add_argument("--out", required=True, type=Path)

    verify = subparsers.add_parser("verify")
    verify.add_argument("--evidence", required=True, type=Path)
    verify.add_argument("--revision", required=True)
    verify.add_argument("--artifact", required=True, type=Path)
    verify.add_argument("--reports-dir", required=True, type=Path)
    args = parser.parse_args(argv)

    try:
        if args.command == "create":
            create_evidence(
                revision=args.revision,
                artifact=args.artifact,
                profile=args.profile,
                device_abi=args.device_abi,
                installed_native_sha256=args.installed_native_sha256,
                package=args.package,
                device_model=args.device_model,
                android_api=args.android_api,
                device_serial=args.device_serial,
                reports=_report_args(args.report),
                output=args.out,
            )
            print(f"Release evidence created: {args.out}")
        else:
            verify_evidence(
                evidence=args.evidence,
                revision=args.revision,
                artifact=args.artifact,
                reports_dir=args.reports_dir,
            )
            print("Release evidence gate: PASS")
    except EvidenceError as error:
        print(f"Release evidence gate: FAIL ({error})", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
