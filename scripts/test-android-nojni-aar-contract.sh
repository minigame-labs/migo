#!/usr/bin/env bash
# The engine-less AAR must be the published AAR minus its engine, and nothing else.
#
# Two AARs under two names is a standing invitation to the failure this gate exists
# to prevent: a consumer integrates `-nojni`, delivers the engine at runtime, and
# gets a classes.jar, an AndroidManifest, or an embedded artifact identity that is
# not the one the engine was cut against. Nothing in a Gradle build would notice --
# the two artifacts would simply have been produced by two builds.
#
# So `-nojni` is not built. It is derived, by deletion, from the AAR the release
# already publishes, and this gate is what makes that claim checkable:
#
#   1. `-nojni` holds no `jni/` entries at all
#   2. every other entry is byte-identical to the published AAR's, in both
#      directions -- nothing dropped, nothing added, nothing rewritten
#   3. `assets/migo/artifacts/**` survives the deletion. This is the point of the
#      split: the AAR that carries no engine still carries the SHA-256 of the
#      engine it expects, which is what lets the SDK verify a host-delivered
#      binary offline instead of trusting whoever served it
#   4. every removed file reappears in exactly one engine archive, byte for byte,
#      so the split loses nothing
#   5. each archive's libmigo.so hashes to what its slice manifest declares --
#      the check the device will make, made here first
#
# Everything is compared by streaming digest rather than by reading entries into
# memory. A debug libmigo.so is ~374 MB and this gate compares each one twice; the
# obvious implementation needs three quarters of a gigabyte of RAM per ABI and
# fails on a CI runner rather than on anything it was meant to catch.
#
# Usage: test-android-nojni-aar-contract.sh <published.aar> <nojni.aar> <archive.tar.gz>...
set -euo pipefail

if [[ $# -lt 3 ]]; then
    echo "usage: $0 <published-aar> <nojni-aar> <engine-archive>..." >&2
    exit 2
fi

python3 - "$@" <<'PY'
from __future__ import annotations

import hashlib
import json
import pathlib
import sys
import tarfile
import zipfile

JNI_PREFIX = "jni/"
MANIFEST_PREFIX = "assets/migo/artifacts/"
ENGINE_BINARY = "libmigo.so"
ARCH_SEGMENT = {"arm64-v8a": "arm64", "x86_64": "x86_64"}
CHUNK = 1 << 20

published = pathlib.Path(sys.argv[1])
nojni = pathlib.Path(sys.argv[2])
archives = [pathlib.Path(p) for p in sys.argv[3:]]

failures: list[str] = []
checks = 0


def check(condition: bool, message: str) -> bool:
    global checks
    checks += 1
    if not condition:
        failures.append(message)
    return condition


def digest(stream) -> str:
    out = hashlib.sha256()
    while True:
        chunk = stream.read(CHUNK)
        if not chunk:
            break
        out.update(chunk)
    return out.hexdigest()


for path in [published, nojni] + archives:
    if not path.is_file():
        print(f"Android -nojni AAR contract: no such file: {path}", file=sys.stderr)
        raise SystemExit(1)

with zipfile.ZipFile(published) as full, zipfile.ZipFile(nojni) as shell:
    full_entries = {info.filename for info in full.infolist()}
    shell_entries = {info.filename for info in shell.infolist()}

    check(
        not any(name.startswith(JNI_PREFIX) for name in shell_entries),
        f"{nojni.name} still carries jni/ entries: "
        f"{sorted(n for n in shell_entries if n.startswith(JNI_PREFIX))}",
    )

    removed = {name for name in full_entries if name.startswith(JNI_PREFIX)}
    check(
        bool(removed),
        f"{published.name} carries no jni/ entries, so nothing was split out. Deriving "
        "a -nojni AAR from an engine-less AAR publishes two names for one artifact",
    )

    expected_kept = full_entries - removed
    check(
        expected_kept == shell_entries,
        f"entry sets differ beyond jni/: only in {published.name}="
        f"{sorted(expected_kept - shell_entries)}, "
        f"only in {nojni.name}={sorted(shell_entries - expected_kept)}",
    )

    for name in sorted(expected_kept & shell_entries):
        if name.endswith("/"):
            continue
        checks += 1
        with full.open(name) as a, shell.open(name) as b:
            if digest(a) != digest(b):
                failures.append(
                    f"{name} differs between {published.name} and {nojni.name}; the "
                    "engine-less AAR must be a deletion, not a second build"
                )

    check(
        any(n.startswith(MANIFEST_PREFIX + "slices/") for n in shell_entries),
        f"{nojni.name} carries no {MANIFEST_PREFIX}slices/*.json. Without them the SDK "
        "has nothing to verify a host-delivered engine against and refuses to load it, "
        "which makes the engine-less AAR unusable for the one purpose it has",
    )

    abis = sorted(
        {
            name[len(JNI_PREFIX):].split("/", 1)[0]
            for name in removed
            if "/" in name[len(JNI_PREFIX):]
        }
    )
    unclaimed = set(archives)
    for abi in abis:
        arch = ARCH_SEGMENT.get(abi)
        if not check(arch is not None, f"jni/{abi} has no arch segment in the asset scheme"):
            continue
        matching = [p for p in archives if p.name.endswith(f"-{arch}.tar.gz")]
        if not check(
            len(matching) == 1,
            f"expected exactly one engine archive for jni/{abi} among "
            f"{[p.name for p in archives]}, found {[p.name for p in matching]}",
        ):
            continue
        archive = matching[0]
        unclaimed.discard(archive)

        with tarfile.open(archive, "r:gz") as tar:
            roots = set()
            member_digests: dict[str, str] = {}
            member_bytes: dict[str, bytes] = {}
            for member in tar.getmembers():
                if not member.isfile() or "/" not in member.name:
                    continue
                root, leaf = member.name.split("/", 1)
                roots.add(root)
                handle = tar.extractfile(member)
                if leaf in ("slice.json", "README.md"):
                    member_bytes[leaf] = handle.read()
                    member_digests[leaf] = hashlib.sha256(member_bytes[leaf]).hexdigest()
                else:
                    member_digests[leaf] = digest(handle)

        check(
            roots == {archive.name[: -len(".tar.gz")]},
            f"{archive.name} does not unpack into a single "
            f"{archive.name[:-len('.tar.gz')]}/ directory (found {sorted(roots)})",
        )

        for name in sorted(
            n for n in removed if n.startswith(f"{JNI_PREFIX}{abi}/") and not n.endswith("/")
        ):
            leaf = name.rsplit("/", 1)[1]
            if not check(leaf in member_digests, f"{archive.name} does not carry {name}"):
                continue
            with full.open(name) as entry:
                expected = digest(entry)
            check(
                member_digests[leaf] == expected,
                f"{archive.name}:{leaf} differs from {published.name}:{name}",
            )

        if not check(
            ENGINE_BINARY in member_digests, f"{archive.name} carries no {ENGINE_BINARY}"
        ):
            continue
        if not check("slice.json" in member_bytes, f"{archive.name} carries no slice.json"):
            continue
        slice_path = f"{MANIFEST_PREFIX}slices/{abi}.json"
        check(
            slice_path in full_entries
            and member_bytes["slice.json"] == full.read(slice_path),
            f"{archive.name}:slice.json is not the manifest embedded in {published.name}",
        )
        try:
            declared = json.loads(member_bytes["slice.json"])["hashes"]["runtime_binary"]
        except (ValueError, KeyError, TypeError):
            failures.append(f"{archive.name}:slice.json declares no hashes.runtime_binary")
            checks += 1
            continue
        check(
            declared == member_digests[ENGINE_BINARY],
            f"{archive.name}:{ENGINE_BINARY} hashes to {member_digests[ENGINE_BINARY]} but "
            f"its slice.json declares {declared}. A host would fetch this, fail "
            "verification, and have no way to tell whether the engine or the SDK was wrong",
        )

    check(
        not unclaimed,
        f"engine archives matching no ABI in {published.name}: "
        f"{sorted(p.name for p in unclaimed)}",
    )

if failures:
    for line in failures:
        print(f"Android -nojni AAR contract: {line}", file=sys.stderr)
    print(
        f"Android -nojni AAR contract: FAIL ({len(failures)} of {checks} checks)",
        file=sys.stderr,
    )
    raise SystemExit(1)

print(
    f"Android -nojni AAR contract: PASS ({checks} checks over {nojni.name} "
    f"and {len(archives)} archive(s))"
)
PY
