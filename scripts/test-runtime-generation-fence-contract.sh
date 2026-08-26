#!/usr/bin/env bash
# A fenced callback must stamp the generation its producer captured.
#
# A runtime restart replaces the JavaScript isolate but not the Android objects
# around it: a sensor listener stays registered, a camera keeps delivering
# frames, a dialog stays on screen. An event those produce belongs to the runtime
# that is going away, and delivering it to the replacement is cross-talk between
# two isolates. The fence that prevents it is one rule: the generation is
# captured where the producer was created and carried on every event it reports.
#
# Re-reading the current generation at report time satisfies the shape and
# nothing else -- it always matches, and a comparison that always matches is not
# a check. Passing a literal is the same defect written shorter. Neither is
# visible in review as anything but a plausible argument, and no host test can
# catch it: five of the six Android producer groups need a Context or a main
# Looper to construct, so nothing on this machine can build one and restart a
# session behind its back. That is why the property is checked structurally.
#
# The fenced set is DERIVED, and from the handler rather than from the JNI
# signature. A signature is not the authority it looks like: `onVsync(IJ)V` and
# `getConsoleLogs(IJ)Ljava/lang/String;` both put a plain payload long in the
# slot a generation would occupy -- a frame timestamp, a log cursor. Selecting on
# `^\(IJ` would demand a captured token where a frame timestamp belongs, and the
# fix for that failure would be to break vsync. What declares the intent is the
# JNI handler's own parameter list in `inbound.rs`:
# `host_id: jint, generation: jlong`.
#
# So three engine facts are cross-checked against each other rather than one
# being trusted:
#
#   1. the handler names its second JNI parameter `generation: jlong`;
#   2. it converts that parameter with `captured_generation(...)`, which is what
#      turns it into the `Option<NonZeroI64>` the HostCommand carries -- a
#      handler that accepts the parameter and drops it fences nothing;
#   3. the method's `NATIVE_*` descriptor in `profile_contract.rs` begins `(IJ`,
#      because a descriptor that loses its `J` is an argument-shape mismatch that
#      decodes the call frame wrongly rather than failing to compile.
#
# Their disagreement is itself a defect, so each direction is reported. The
# descriptor set is also read the other way round: `(IJ` is the whole surface on
# which a generation can arrive, so every method on it must be a handler this
# parse can see. `inbound.rs` generates some handlers from a macro, and one
# generated that way is invisible here -- which would shrink the fenced set
# silently rather than loudly.
#
# Then, for every fenced method, every `NativeMethods.<name>(` call site in the
# Java library must stamp either a captured token or the explicit unfenced
# constant, and `NativeMethods.<name>` must forward its own `generation`
# parameter rather than substituting one.
#
# `RuntimeGenerationBoundary.UNFENCED` is accepted because it is a real answer:
# an export replying synchronously to a call the live runtime just made captured
# no generation and cannot be stale, so it must never be dropped. It says so at
# the call site, which is the difference between it and a literal `0`.
#
# The receiver of `.generation()` must be a `final RuntimeGenerationBoundary.Token`
# field of the same file. That is stricter than "looks like a token" on purpose:
# a final field can only have been assigned once, at construction, which is
# exactly the captured-at-creation property. Anything re-derived at report time --
# `RuntimeGenerationBoundary.acquire(sessionId).generation()`, a mutable field
# reassigned by a restart listener, a helper that reads the live value -- cannot
# reach the argument through a final field, so it fails without this gate needing
# to enumerate the ways of getting it wrong.
#
# Vacuity is checked in both directions, because an empty scan and a clean scan
# print the same thing: the fenced set must be non-empty, and every fenced method
# must have at least one Java call site. The second is what survives a file being
# renamed or a call being reshaped past the extraction rule; without it this gate
# would go quiet exactly when the code moved.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT_DIR" <<'PY'
from __future__ import annotations

import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1]).resolve()

INBOUND = root / "engine/crates/platform/src/android/jni/inbound.rs"
CONTRACT = root / "engine/crates/platform/src/android/jni/profile_contract.rs"
JAVA_MAIN = root / "platforms/android/library/src/main/java"
NATIVE_METHODS = JAVA_MAIN / "com/migo/runtime/internal/NativeMethods.java"

for required in (INBOUND, CONTRACT, JAVA_MAIN, NATIVE_METHODS):
    if not required.exists():
        print(
            f"ERROR: {required.relative_to(root)} not found; this gate cannot check "
            "anything",
            file=sys.stderr,
        )
        sys.exit(1)


# The reading of both sides -- masking comments and literals so structure can be
# scanned, matching brackets, splitting argument lists, and the descriptor tables
# themselves -- is shared with `test-jni-outbound-signature-contract.sh`. A parser
# bug fixed in one copy and not the other is how two gates end up unequal while
# looking alike.
sys.path.insert(0, str(root / "scripts/lib"))

from jni_source import (  # noqa: E402
    balanced_end,
    descriptor_table,
    line_of,
    mask_java,
    mask_rust,
    normalise,
    spaced,
    split_arguments,
)


# ------------------------------------------------- authority 1: the JNI handlers

inbound_source = INBOUND.read_text(encoding="utf-8")
inbound_masked = mask_rust(inbound_source)

# The ABI string is matched as a blanked literal: masking empties `"system"`,
# which is what every `extern` here declares.
HANDLER = re.compile(r'extern\s+"[^"]*"\s+fn\s+(?P<name>[A-Za-z0-9_]+)\s*(?:<[^>]*>)?\s*\(')

handlers: dict[str, list[tuple[str, str]]] = {}  # name -> [(parameter list, body)]
for match in HANDLER.finditer(inbound_masked):
    parameters_end = balanced_end(inbound_masked, match.end() - 1)
    if parameters_end is None:
        continue
    body_start = inbound_masked.find("{", parameters_end)
    if body_start == -1:
        continue
    body_end = balanced_end(inbound_masked, body_start)
    if body_end is None:
        continue
    # A list, not one entry per name: a `#[cfg(target_os = "android")]` handler and
    # its non-Android stub share a name, and keeping only the last would drop a
    # fenced handler out of the derived set behind whichever of the two came
    # second. That narrows the gate silently, which is the failure it exists to
    # prevent.
    handlers.setdefault(match.group("name"), []).append(
        (
            inbound_masked[match.end() : parameters_end],
            inbound_masked[body_start : body_end + 1],
        )
    )

if not handlers:
    print(
        f"ERROR: parsed no JNI handler out of {INBOUND.relative_to(root)}; the fenced "
        "set would be empty and every check below would pass vacuously",
        file=sys.stderr,
    )
    sys.exit(1)

SECOND_IS_GENERATION = re.compile(r"host_id\s*:\s*jint\s*,\s*generation\s*:\s*jlong")
CAPTURES = re.compile(r"captured_generation\s*\(\s*generation\s*\)")

fenced: set[str] = set()
errors: list[str] = []

for name, definitions in sorted(handlers.items()):
    # Any definition declaring the parameter fences the name, and any definition
    # converting it satisfies the conversion: a target-specific stub shares the JNI
    # signature -- it must, since both targets register the same descriptor -- but
    # it is not the definition that does the work.
    declares = any(SECOND_IS_GENERATION.search(spaced(parameters)) for parameters, _ in definitions)
    captures = any(CAPTURES.search(spaced(body)) for _, body in definitions)
    if declares:
        fenced.add(name)
        if not captures:
            errors.append(
                f"engine: `{name}` takes a `generation: jlong` and never converts it "
                "with `captured_generation(generation)`, so the value its producer "
                "captured is dropped on the way to the HostCommand and the callback is "
                "unfenced while looking fenced"
            )
    elif captures:
        errors.append(
            f"engine: `{name}` calls `captured_generation(generation)` but does not "
            "declare `host_id: jint, generation: jlong` as its first two JNI "
            "parameters, so the fenced set derived here cannot see it"
        )

if not fenced:
    print(
        f"ERROR: no handler in {INBOUND.relative_to(root)} declares `host_id: jint, "
        "generation: jlong`; either every fence was removed or this parse has stopped "
        "matching, and the gate would pass vacuously",
        file=sys.stderr,
    )
    sys.exit(1)

# ---------------------------------------- authority 2: the JNI method descriptors

descriptors = descriptor_table(CONTRACT.read_text(encoding="utf-8"), "NATIVE_")

if not descriptors:
    print(
        f"ERROR: parsed no NATIVE_* descriptor out of {CONTRACT.relative_to(root)}; the "
        "signature cross-check would pass vacuously",
        file=sys.stderr,
    )
    sys.exit(1)

for name in sorted(fenced):
    signature = descriptors.get(name)
    if signature is None:
        errors.append(
            f"engine: `{name}` is a fenced handler with no NATIVE_* descriptor in "
            "profile_contract.rs, so no product profile registers it and the callback "
            "cannot fire at all"
        )
    elif not signature.startswith("(IJ"):
        errors.append(
            f"engine: `{name}` takes a `generation: jlong` but its descriptor is "
            f"`{signature}`, which does not begin `(IJ`; JNI reads the arguments by the "
            "descriptor, so the call frame is decoded wrongly at registration"
        )

# The other direction, which is what keeps the derivation honest. A generation
# has to arrive in the second JNI slot, so `(IJ` is the whole risk surface, and
# every method on it must be a handler this parse can see and classify. Three of
# them carry a payload long rather than a generation -- a frame timestamp, a
# refresh period, a log cursor -- and being visible is what lets them be
# classified as unfenced instead of guessed at. `inbound.rs` also has a macro that
# generates handlers, and a handler generated by one is invisible here: this is
# the check that says so, rather than the fenced set quietly shrinking by one.
for name, signature in sorted(descriptors.items()):
    if signature.startswith("(IJ") and name not in handlers:
        errors.append(
            f"engine: `{name}` has descriptor `{signature}`, so a long arrives in the "
            "slot a runtime generation occupies, but no `extern` handler by that name is "
            "visible to this parse -- generated by a macro, or renamed. An invisible "
            "handler cannot be classified, so a fence on it would go unchecked"
        )

# ----------------------------------------------------- the Java producer surface

java_sources = sorted(JAVA_MAIN.rglob("*.java"))
if not java_sources:
    print(
        f"ERROR: no Java source under {JAVA_MAIN.relative_to(root)}; every call-site "
        "check would pass vacuously",
        file=sys.stderr,
    )
    sys.exit(1)

TOKEN_FIELD = re.compile(
    r"final\s+RuntimeGenerationBoundary\.Token\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*[;=]"
)
UNFENCED = "RuntimeGenerationBoundary.UNFENCED"

call_sites: dict[str, int] = {name: 0 for name in fenced}

for source_path in java_sources:
    source = source_path.read_text(encoding="utf-8")
    if "NativeMethods." not in source:
        continue
    masked = mask_java(source)
    captured_fields = set(TOKEN_FIELD.findall(masked))
    accepted = {UNFENCED}
    for field in captured_fields:
        accepted.add(f"{field}.generation()")
        accepted.add(f"this.{field}.generation()")
    relative = source_path.relative_to(root)

    for match in re.finditer(r"\bNativeMethods\.([A-Za-z0-9_]+)\s*\(", masked):
        name = match.group(1)
        if name not in fenced:
            continue
        open_paren = match.end() - 1
        close_paren = balanced_end(masked, open_paren)
        if close_paren is None:
            errors.append(
                f"{relative}:{line_of(source, open_paren)}: the argument list of "
                f"`NativeMethods.{name}(` has no closing parenthesis this gate can "
                "find, so its generation argument was never checked"
            )
            continue
        arguments = split_arguments(
            source[open_paren + 1 : close_paren], masked[open_paren + 1 : close_paren]
        )
        call_sites[name] += 1
        if len(arguments) < 2:
            errors.append(
                f"{relative}:{line_of(source, open_paren)}: `NativeMethods.{name}` is "
                "fenced and takes a generation as its second argument, but this call "
                f"passes {len(arguments)}"
            )
            continue
        stamped = normalise(arguments[1])
        if stamped in accepted:
            continue
        if stamped.endswith(".generation()"):
            reason = (
                f"`{stamped}` does not read a `final RuntimeGenerationBoundary.Token` "
                "field of this file, so it is not the generation captured when the "
                "producer was created; a generation re-read at report time always "
                "matches the live runtime and checks nothing"
            )
        else:
            reason = (
                f"`{stamped}` is neither a captured token nor "
                "`RuntimeGenerationBoundary.UNFENCED`; an event stamped with anything "
                "else is delivered to whichever runtime happens to be live"
            )
        errors.append(f"{relative}:{line_of(source, open_paren)}: {reason}")

for name in sorted(fenced):
    if call_sites[name] == 0:
        errors.append(
            f"`NativeMethods.{name}` is fenced in the engine and this gate found no "
            f"call to it under {JAVA_MAIN.relative_to(root)}: either the callback has "
            "no producer and can never fire, or its call site no longer matches the "
            "rule that extracts it -- and an unmatched call site is an unchecked one"
        )

# --------------------------------------------- the forward through NativeMethods

native_source = NATIVE_METHODS.read_text(encoding="utf-8")
native_masked = mask_java(native_source)
native_relative = NATIVE_METHODS.relative_to(root)

for name in sorted(fenced):
    declaration = re.search(
        rf"\bstatic\s+\w[\w<>\[\].]*\s+{re.escape(name)}\s*\(", native_masked
    )
    if declaration is None:
        errors.append(
            f"{native_relative}: no `static ... {name}(` wrapper for the fenced "
            f"callback `{name}`; nothing in the Java library can reach it"
        )
        continue
    at = line_of(native_source, declaration.start())
    parameters_end = balanced_end(native_masked, declaration.end() - 1)
    if parameters_end is None:
        errors.append(
            f"{native_relative}:{at}: the parameter list of `{name}` has no closing "
            "parenthesis this gate can find"
        )
        continue
    parameters = split_arguments(
        native_source[declaration.end() : parameters_end],
        native_masked[declaration.end() : parameters_end],
    )
    if len(parameters) < 2 or normalise(parameters[1]) != "longgeneration":
        second = normalise(parameters[1]) if len(parameters) > 1 else "<none>"
        errors.append(
            f"{native_relative}:{at}: `{name}` is fenced in the engine, so its second "
            f"parameter must be `long generation`; it is `{second}`"
        )
        continue

    body_start = native_masked.find("{", parameters_end)
    body_end = balanced_end(native_masked, body_start) if body_start != -1 else None
    if body_end is None:
        errors.append(
            f"{native_relative}:{at}: the body of `{name}` has no closing brace this "
            "gate can find"
        )
        continue
    forward = re.search(
        rf"\bNativeBridge\.{re.escape(name)}\s*\(", native_masked[body_start : body_end + 1]
    )
    if forward is None:
        errors.append(
            f"{native_relative}:{at}: `{name}` never calls `NativeBridge.{name}`, so "
            "the generation it was given reaches no JNI boundary"
        )
        continue
    forward_open = body_start + forward.end() - 1
    forward_close = balanced_end(native_masked, forward_open)
    if forward_close is None:
        errors.append(
            f"{native_relative}:{line_of(native_source, forward_open)}: the argument "
            f"list of `NativeBridge.{name}(` has no closing parenthesis this gate can "
            "find"
        )
        continue
    forwarded = split_arguments(
        native_source[forward_open + 1 : forward_close],
        native_masked[forward_open + 1 : forward_close],
    )
    if len(forwarded) < 2 or normalise(forwarded[1]) != "generation":
        second = normalise(forwarded[1]) if len(forwarded) > 1 else "<none>"
        errors.append(
            f"{native_relative}:{line_of(native_source, forward_open)}: `{name}` "
            f"forwards `{second}` to `NativeBridge.{name}` instead of its own "
            "`generation` parameter, so every producer's captured value is replaced "
            "here and the fences upstream of it are decoration"
        )

if errors:
    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    print(
        f"Runtime generation fence contract: FAIL ({len(errors)} violation(s) across "
        f"{len(fenced)} fenced callbacks)",
        file=sys.stderr,
    )
    sys.exit(1)

print(
    f"Runtime generation fence contract: PASS ({len(fenced)} fenced callbacks derived "
    f"from {sum(len(d) for d in handlers.values())} JNI handler definitions, "
    f"{sum(call_sites.values())} Java call sites all "
    "stamping a captured token or UNFENCED, every wrapper forwarding its own generation)"
)
PY
