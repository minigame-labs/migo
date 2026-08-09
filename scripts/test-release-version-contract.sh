#!/usr/bin/env bash
# One repository, one release version.
#
# The version is a fact about a set of bytes, and it is stated in four build
# systems: a Cargo manifest, a Gradle `versionName`, CMake package files, and the
# packaging scripts that write archive manifests. Each was written independently
# and each drifted:
#
#   * the Android AAR reported `0.9.0` while the Android C-API SDK built beside it
#     defaulted to `0.1.0` -- one platform disagreeing with itself, because two
#     scripts build the two artifacts and only one had been told;
#   * the Windows SDK hardcoded `0.1.1` and *discarded* the version it had just
#     read, so it announced a version no other platform had heard of;
#   * the Linux and HarmonyOS SDKs derived theirs from `crates/capi/Cargo.toml`,
#     HarmonyOS with a silent `0.1.0` fallback -- a fallback that labels a package
#     with a version nobody chose, on the path where being wrong is permanent.
#
# `release/VERSION` is now the single source, and this gate is what makes that
# true rather than merely intended. Nothing here bumps or proposes a version: it
# checks that every consumer derives from the one file, and that the one literal
# no format allows to be derived agrees with it.
#
# **Cargo is the exception, and it is deliberate.** A manifest takes a literal, so
# `[workspace.package] version` cannot read a file. It is therefore a mirror, and
# the mirror is checked here. Every workspace member inherits it with
# `version.workspace = true`, so there is one literal rather than sixteen.
#
# **What must NOT be unified**, checked so a later change cannot quietly fold them
# in:
#
#   * `platforms/openharmony/entry/` and `AppScope/` are a demo *application*
#     (`"type": "entry"`, `bundleName com.migo.ohoshost`, vendor `example`), not
#     the shipped library. Its version is its own.
#   * `tests/c_host/android/` is a test application
#     (`com.android.application`), not the SDK.
#   * `include/migo/types.h`'s `MIGO_ABI_VERSION_*` are a protocol number that
#     moves when the ABI changes shape, which is a different question from which
#     release a binary came from. Folding the two would make an ABI-compatible
#     release look like a breaking one, or worse, the reverse.
#   * `adapter/package.json` is a separately publishable npm package layered on the
#     JS API surface, with its own consumers and cadence.
#
# The release version only ever moves forward, and that constraint predates this
# gate: `0.1.0` shipped a Windows DLL that could attach no surface kind, and the
# fix went out as `0.1.1` so those bytes would not arrive under a version a
# consumer already held. A single forward-moving source keeps that guarantee; a
# detached literal per platform is what lost it.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT_DIR" <<'PY'
from __future__ import annotations

import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1]).resolve()

SOURCE = root / "release/VERSION"
if not SOURCE.is_file():
    print(
        f"ERROR: {SOURCE.relative_to(root)} not found; there is no release-version "
        "source for anything to derive from",
        file=sys.stderr,
    )
    sys.exit(1)

raw = SOURCE.read_text(encoding="utf-8")
version = raw.strip()
if not version:
    print(f"ERROR: {SOURCE.relative_to(root)} is empty", file=sys.stderr)
    sys.exit(1)

errors: list[str] = []

# Semantic Versioning 2.0.0. The same shape `gen-android-package-metadata.py` and
# `gen-linux-package-metadata.py` already enforce on what they are handed, applied
# to the source they will be handed it from -- a source those two would reject is
# a build that fails at packaging time instead of here.
SEMVER = re.compile(
    r"^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
    r"(?:-(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)"
    r"(?:\.(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*))*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)
if not SEMVER.fullmatch(version):
    errors.append(
        f"release/VERSION is `{version}`, which is not Semantic Versioning 2.0.0; the "
        "package metadata generators reject it, so every packaging run would fail at "
        "the point where the version is written rather than here"
    )

if raw != version + "\n":
    errors.append(
        "release/VERSION must be the version and one trailing newline, so every reader "
        "gets the same string: a bash `$(<file)`, a Gradle `text.trim()` and a Python "
        f"`read_text()` do not agree about surrounding whitespace ({raw!r})"
    )

# ------------------------------------------------- the one literal no format allows

WORKSPACE = root / "engine/Cargo.toml"
workspace_text = WORKSPACE.read_text(encoding="utf-8")
package_section = re.search(
    r"^\[workspace\.package\]\s*$(.*?)^\[", workspace_text, re.M | re.S
)
declared = (
    re.search(r'^version\s*=\s*"([^"]+)"', package_section.group(1), re.M)
    if package_section
    else None
)
if declared is None:
    errors.append(
        "engine/Cargo.toml has no `[workspace.package] version`; without it every "
        "member carries its own literal and the mirror this gate checks does not exist"
    )
elif declared.group(1) != version:
    errors.append(
        f"engine/Cargo.toml `[workspace.package] version` is `{declared.group(1)}` and "
        f"release/VERSION is `{version}`. Cargo cannot read a file, so this literal is "
        "a mirror and the two move together or the SDKs built from this tree disagree "
        "about which release they are"
    )

# Every member must inherit rather than restate. A member that reintroduces its own
# literal is the sixteen-literal state coming back one crate at a time.
members = re.search(r"^members\s*=\s*\[(.*?)\]", workspace_text, re.M | re.S)
if members is None:
    errors.append("engine/Cargo.toml has no `members` list; this gate cannot find the crates")
else:
    paths = re.findall(r'"([^"]+)"', members.group(1))
    if not paths:
        errors.append(
            "engine/Cargo.toml lists no workspace members; the inheritance check would "
            "pass over nothing"
        )
    for member in paths:
        manifest = root / "engine" / member / "Cargo.toml"
        if not manifest.is_file():
            errors.append(f"engine/{member}/Cargo.toml is a listed member with no manifest")
            continue
        text = manifest.read_text(encoding="utf-8")
        package = re.search(r"^\[package\]\s*$(.*?)(?=^\[|\Z)", text, re.M | re.S)
        body = package.group(1) if package else text
        literal = re.search(r'^version\s*=\s*"([^"]+)"', body, re.M)
        if literal is not None:
            errors.append(
                f"engine/{member}/Cargo.toml restates `version = \"{literal.group(1)}\"` "
                "instead of `version.workspace = true`; a per-crate literal is what this "
                "gate exists to keep from coming back"
            )
        elif "version.workspace" not in body:
            errors.append(
                f"engine/{member}/Cargo.toml declares no version at all, neither a "
                "literal nor `version.workspace = true`"
            )

# --------------------------------------------------- every other consumer derives

# A consumer is checked by what it *reads*, not by comparing a value: a script that
# names the source cannot drift from it, and one that hardcodes a version cannot be
# made to agree by anything but an edit. So the assertion is textual and the failure
# names the source it should be reading.
DERIVES = {
    "platforms/android/library/build.gradle": (
        "migoReleaseVersion()",
        "the AAR's `versionName`, which reaches an embedder through BuildInfo.VERSION",
    ),
    "scripts/build-linux-sdk.sh": ("read_release_version", "the Linux SDK's CMake package files"),
    "scripts/build-ohos-sdk.sh": ("read_release_version", "the HarmonyOS SDK's CMake package files"),
    "scripts/build-windows-sdk.sh": ("read_release_version", "the Windows SDK's CMake package files"),
    "scripts/build-android-sdk.sh": ("release/VERSION", "the Android C-API SDK's package metadata"),
}

for relative, (marker, what) in sorted(DERIVES.items()):
    path = root / relative
    if not path.is_file():
        errors.append(f"{relative} not found; it writes {what} and this gate cannot check it")
        continue
    if marker not in path.read_text(encoding="utf-8"):
        errors.append(
            f"{relative} no longer reads the release version (`{marker}` is gone). It "
            f"writes {what}, so whatever it uses instead is what ships"
        )

# A version literal that looks like a release version, on a line that is *setting*
# a version, is the drift itself.
#
# Only lines naming a version are scanned, which is what keeps this from rotting: a
# dependency coordinate carries a version number too (`junit:junit:4.13.2`,
# `org.pitest:pitest:1.16.1`) and bumping one is routine. An allowlist of those
# would need editing every time a dependency moved, and a gate that has to be
# edited to stay green is a gate people edit without reading.
LITERAL = re.compile(r'(?<![\w.-])\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?![\w.-])')
SETS_A_VERSION = re.compile(r"version", re.I)
ALLOWED_LITERALS = {
    # Gradle's fallback for a task that reads a `versionName` which is somehow unset.
    "platforms/android/library/build.gradle": {"0.0.0"},
}
for relative in sorted(DERIVES):
    path = root / relative
    if not path.is_file():
        continue
    allowed = ALLOWED_LITERALS.get(relative, set())
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        code = line.split("#", 1)[0] if relative.endswith(".sh") else line
        if "//" in code:
            code = code.split("//", 1)[0]
        if not SETS_A_VERSION.search(code):
            continue
        for found in LITERAL.findall(code):
            if found == version or found in allowed:
                continue
            errors.append(
                f"{relative}:{number}: `{found}` is a version literal in a file that "
                f"should derive from release/VERSION (`{version}`): {line.strip()}"
            )

# ------------------------------------------------ what must stay independent

INDEPENDENT = {
    "platforms/openharmony/entry/oh-package.json5": "a demo application, not the shipped library",
    "tests/c_host/android/build.gradle": "a test application, not the SDK",
}
for relative, why in sorted(INDEPENDENT.items()):
    path = root / relative
    if not path.is_file():
        # Not an error: these are allowed to be removed. The check is that they are
        # not *folded into* the release version, and a file that is gone cannot be.
        continue
    if "release/VERSION" in path.read_text(encoding="utf-8"):
        errors.append(
            f"{relative} now derives from release/VERSION, but it is {why}. Its version "
            "is its own, and tying it to the release version makes a demo bump look like "
            "a release"
        )

if errors:
    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    print(
        f"Release version contract: FAIL ({len(errors)} violation(s) against "
        f"release/VERSION = {version})",
        file=sys.stderr,
    )
    sys.exit(1)

print(
    f"Release version contract: PASS (release/VERSION = {version}; the Cargo workspace "
    f"mirror agrees, {len(paths)} members inherit it, {len(DERIVES)} build consumers "
    f"derive from it, {len(INDEPENDENT)} independent versions left alone)"
)
PY
