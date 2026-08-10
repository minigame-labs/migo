#!/usr/bin/env bash
# A Java method the engine calls must have the shape the engine calls it with.
#
# Every outbound call resolves against one class -- `NativeExports`, through
# `JAVA_METHOD_CACHE` -- by name *and descriptor*. The descriptors live in
# `profile_contract.rs`'s `JAVA_*` tables, and the methods live in Java, so the
# two are one contract written twice.
#
# A disagreement compiles. Both sides are valid on their own: the Rust table is
# string literals, the Java method is a method, and neither refers to the other in
# a way a compiler can check. What fails is `GetStaticMethodID` on a real device,
# once, at the moment the feature is first used -- the most expensive place a
# mismatch can surface and the one furthest from the edit that caused it.
#
# This was measured rather than assumed. Widening `getSystemSettingInfoBytes` to
# `(I)[B` on the Rust side while leaving the Java method no-arg passes the product
# profile contract, both R8 root checks, the host API contract and `javac`. The
# profile contract compares method *names*; it was never a signature check, and
# `test-camera-frame-jni-contract.sh` exists because that gap was found the hard
# way for one method out of a hundred and twenty-six. This is that gate for all of
# them.
#
# Reference types are compared by simple name, because that is what the Java
# source spells under its imports: `Ljava/lang/String;` against `String`. A Java
# declaration that writes a type out in full is reduced the same way, so the two
# spellings of one type cannot read as a mismatch.
#
# The inbound direction is not checked here. It has a stronger guarantee already:
# `registration.rs` binds each `NATIVE_*` name to a Rust `extern "system" fn`, and
# a handler whose parameters disagree with its descriptor is caught by
# `test-runtime-generation-fence-contract.sh` for the generation slot and by the
# registration itself for the rest.
#
# Vacuity is checked in both directions: the descriptor set must be non-empty, the
# Java declarations must be non-empty, and every descriptor must find its method.
# An empty scan and a clean scan print the same thing otherwise.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT_DIR" <<'PY'
from __future__ import annotations

import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1]).resolve()
sys.path.insert(0, str(root / "scripts/lib"))

from jni_source import (  # noqa: E402
    balanced_end,
    decode_descriptor,
    descriptor_table,
    line_of,
    mask_java,
    normalise,
    spaced,
    split_arguments,
)

CONTRACT = root / "engine/crates/platform/src/android/jni/profile_contract.rs"
EXPORTS = (
    root / "platforms/android/library/src/main/java/com/migo/runtime/internal/NativeExports.java"
)

for required in (CONTRACT, EXPORTS):
    if not required.is_file():
        print(
            f"ERROR: {required.relative_to(root)} not found; this gate cannot check anything",
            file=sys.stderr,
        )
        sys.exit(1)

descriptors = descriptor_table(CONTRACT.read_text(encoding="utf-8"), "JAVA_")
if not descriptors:
    print(
        f"ERROR: parsed no JAVA_* descriptor out of {CONTRACT.relative_to(root)}; either "
        "the tables were renamed or this parse has stopped matching, and the gate would "
        "pass vacuously",
        file=sys.stderr,
    )
    sys.exit(1)

exports_source = EXPORTS.read_text(encoding="utf-8")
exports_masked = mask_java(exports_source)
exports_relative = EXPORTS.relative_to(root)

# `public static` only: JNI resolves a static method on the class, and a private
# helper sharing a name with an exported method must not be able to answer for it.
DECLARATION = re.compile(
    r"\bpublic\s+static\s+(?P<returns>[\w.$]+(?:\s*\[\s*\])*)\s+(?P<name>[A-Za-z_$][\w$]*)\s*\("
)

declarations: dict[str, list[tuple[int, str, list[str]]]] = {}
for match in DECLARATION.finditer(exports_masked):
    close = balanced_end(exports_masked, match.end() - 1)
    if close is None:
        continue
    parameters = split_arguments(
        exports_source[match.end() : close], exports_masked[match.end() : close]
    )
    declarations.setdefault(match.group("name"), []).append(
        (
            line_of(exports_source, match.start()),
            normalise(match.group("returns")),
            [part for part in parameters if part.strip()],
        )
    )

if not declarations:
    print(
        f"ERROR: parsed no `public static` method out of {exports_relative}; every "
        "descriptor would look unimplemented and the gate would fail for the wrong "
        "reason",
        file=sys.stderr,
    )
    sys.exit(1)


_PARAMETER = re.compile(
    r"(?P<type>[\w.$]+(?:\s*\[\s*\])*)\s+(?P<name>[\w$]+)(?P<tail>(?:\s*\[\s*\])*)"
)


def declared_type(parameter: str) -> str:
    """The type of one Java parameter, reduced the way a descriptor decodes to.

    Annotations and `final` are dropped, a fully-qualified name is cut to its
    simple name, and `String[] x` and `String x[]` are one type. The whitespace
    between the type and the name is load-bearing and must survive to here --
    collapsing it makes `int sessionId` one token and the split unrecoverable.
    Generics have no place in a JNI signature, since erasure means the descriptor
    could not describe one, so a parameter carrying one is reported rather than
    guessed at.
    """
    text = spaced(re.sub(r"@[\w.$]+(\([^)]*\))?", " ", parameter)).strip()
    if "<" in text:
        raise ValueError(f"parameter {parameter.strip()!r} is generic")
    text = re.sub(r"^final\s+", "", text)
    match = _PARAMETER.fullmatch(text)
    if match is None:
        raise ValueError(f"parameter {parameter.strip()!r} has no readable type")
    declared = normalise(match.group("type"))
    arrays = normalise(match.group("tail")).count("[]")
    while declared.endswith("[]"):
        arrays += 1
        declared = declared[:-2]
    return declared.rsplit(".", 1)[-1] + "[]" * arrays


errors: list[str] = []

for name, descriptor in sorted(descriptors.items()):
    try:
        expected_parameters, expected_return = decode_descriptor(descriptor)
    except ValueError as undecodable:
        errors.append(
            f"{CONTRACT.relative_to(root)}: `{name}` has descriptor `{descriptor}`, which "
            f"this gate cannot decode ({undecodable}); an undecodable descriptor is one "
            "nothing checks"
        )
        continue

    candidates = declarations.get(name)
    if not candidates:
        errors.append(
            f"{exports_relative}: no `public static` method named `{name}`, but the "
            f"profile contract registers it as `{descriptor}`; the engine resolves it by "
            "name at startup and the call fails on the device"
        )
        continue

    matched = False
    reports: list[str] = []
    for at, returns, parameters in candidates:
        try:
            actual = [declared_type(parameter) for parameter in parameters]
        except ValueError as unreadable:
            reports.append(f"line {at}: {unreadable}")
            continue
        actual_return = returns.rsplit(".", 1)[-1]
        if actual == expected_parameters and actual_return == expected_return:
            matched = True
            break
        reports.append(
            f"line {at}: declares ({', '.join(actual) or 'no parameters'}) -> "
            f"{actual_return}"
        )
    if matched:
        continue

    wanted = f"({', '.join(expected_parameters) or 'no parameters'}) -> {expected_return}"
    errors.append(
        f"{exports_relative}: `{name}` is registered as `{descriptor}`, which is "
        f"{wanted}, and no declaration matches -- " + "; ".join(reports)
    )

if errors:
    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    print(
        f"JNI outbound signature contract: FAIL ({len(errors)} of {len(descriptors)} "
        "registered Java methods do not have the shape the engine calls them with)",
        file=sys.stderr,
    )
    sys.exit(1)

print(
    f"JNI outbound signature contract: PASS ({len(descriptors)} registered Java methods, "
    f"each matching a `public static` declaration among {len(declarations)} in "
    f"{exports_relative.name})"
)
PY
