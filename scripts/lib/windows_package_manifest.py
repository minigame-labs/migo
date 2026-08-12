#!/usr/bin/env python3
"""Verify a staged Windows SDK package against its own package manifest.

Called by scripts/test-windows-sdk-contract.sh. Lives here rather than as a
heredoc inside that gate because it is 60 lines of structural checking, and
because the gate already shells out to Python for the same reason elsewhere.

What it establishes, and why each one is worth a check:

* The manifest exists and declares the schema this reader understands. Every
  other platform's build script writes one; Windows did not, which is why
  scripts/package-sdk.sh refuses a Windows prefix outright and why the published
  migo-windows-x86_64.tar.gz was a `tar` typed by hand rather than the
  reproducible path. The attestation binds the package bytes to this file.

* snapshot_policy is pinned to "none" rather than merely present.
  runtime-v8/build.rs embeds a V8 startup snapshot only when
  target_os == "android", so a Windows package genuinely has none. Pinning means
  that if that ever changes, this fails and forces the manifest to be corrected in
  the same commit instead of the package quietly under-claiming what it contains.

* bin/ holds exactly the DLLs the manifest names. A Windows consumer has to
  redistribute these and the process loads them by name, so both directions are
  defects: a name with no file is a promise about something nobody receives, and a
  file with no name is a DLL the consumer will not know to ship. This is the check
  that turns "the ANGLE directory was incomplete" from a silent shipping defect
  into a failure.

* Every hash the manifest states matches the bytes on disk, for the runtime DLLs
  and for the artifacts map. A manifest that describes a different build than the
  one beside it is worse than no manifest, because the attestation makes it look
  authoritative.

Usage: windows_package_manifest.py <manifest-path> <package-prefix>
Prints one line per problem and exits 1; exits 0 silently when the package agrees
with its manifest.
"""

from __future__ import annotations

import hashlib
import json
import pathlib
import sys

SCHEMA = "migo-windows-package-manifest/v1"


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: windows_package_manifest.py <manifest-path> <package-prefix>")
        return 2

    manifest_path = pathlib.Path(sys.argv[1])
    prefix = pathlib.Path(sys.argv[2])
    problems: list[str] = []

    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except ValueError as error:
        print(f"the manifest is not valid JSON: {error}")
        return 1

    if manifest.get("schema") != SCHEMA:
        problems.append(f"schema is {manifest.get('schema')!r}, expected {SCHEMA!r}")

    if manifest.get("snapshot_policy") != "none":
        problems.append(
            f"snapshot_policy is {manifest.get('snapshot_policy')!r}; expected 'none' "
            "(build.rs embeds a snapshot for android targets only)"
        )

    declared: dict[str, object] = {}
    for entry in manifest.get("runtime_dependencies") or []:
        if not isinstance(entry, dict) or "file" not in entry:
            problems.append("a runtime_dependencies entry has no file")
            continue
        declared[entry["file"]] = entry.get("sha256")
    if not declared:
        problems.append(
            "runtime_dependencies is empty, so the package names no DLLs for a consumer "
            "to redistribute"
        )

    on_disk = {f"bin/{item.name}" for item in (prefix / "bin").glob("*.dll")}
    for extra in sorted(on_disk - set(declared)):
        problems.append(f"{extra} is in the package but the manifest does not name it")
    for absent in sorted(set(declared) - on_disk):
        problems.append(f"the manifest names {absent} but it is not in the package")

    checked = dict(declared)
    checked.update(manifest.get("artifacts") or {})
    for name, expected in sorted(checked.items()):
        path = prefix / name
        if not path.is_file():
            if name not in declared:
                problems.append(f"the manifest's artifacts name {name}, not in the package")
            continue
        actual = sha256(path)
        if actual != expected:
            problems.append(
                f"{name} hashes to {actual} but the manifest claims {expected}"
            )

    for problem in problems:
        print(problem)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
