#!/usr/bin/env bash
# The renderer's host surface kind and the C ABI's attachable kind must be the
# same kind, per Apple platform.
#
# THE DRIFT THIS EXISTS TO CATCH is a disagreement that every existing test is
# built to tolerate. Two places state which layer kind an Apple build is about:
#
#   platforms/apple/Sources/MigoAppleRenderer/MigoEngineCapabilities.swift
#       `hostSurfaceKind`, selected with `#if os(...)` -- what this renderer
#       presents into.
#   engine/crates/capi/src/platform/apple.rs
#       `OWN_LAYER_KIND`, selected with `#[cfg(target_os = ...)]` -- what the
#       library will accept, and what `migo_query_capabilities` advertises.
#
# They are DELIBERATELY independent: `preflight()` compares them at run time, and
# that comparison is the whole reason a host can find out it is about to pass
# something the library will refuse. Independence is the design. Agreement is the
# invariant, and nothing was checking it.
#
# `MigoEngineCapabilitiesTests.testThePreflightVerdictFollowsTheLiveCapabilityMask`
# cannot catch it, on purpose: it is written as an implication, so a disagreement
# takes the `else` branch and the test passes. What a disagreement actually does
# is tell a host "this surface kind is not attachable" on a platform that
# attaches it perfectly well, whereupon the host declines to render and nothing
# anywhere is red. `testTheRendererAndTheLibraryAgreeOnThisHostsSurfaceKind` now
# asserts it directly, but only on the macOS leg of apple-sdk.yml -- which runs
# weekly and on demand, not on pull requests. This is the same fact checked where
# every other Apple fact is checked: on Linux, on every change.
#
# It became load-bearing this round. Before the Apple platform module existed the
# mask was zero for every Apple target, so the two sides could not usefully
# agree; the renderer's own tests were written to avoid pinning that absence.
#
# WHY THE PLATFORM LIST IS DERIVED. Which Apple `target_os` values matter is a
# question about what the SDK builds, so it is asked of
# scripts/build-apple-sdk.sh and of rustc, exactly as
# scripts/test-apple-egl-loader-name-contract.sh asks it. Deriving it is what
# makes the check total: a Swift branch could otherwise be deleted and the
# comparison would simply stop covering that platform. Re-derived here rather
# than shared with that gate, following the recipe/pin pair, which each ask
# build-apple-sdk.sh the same questions independently.
#
# Host-only: it reads two source files and asks two programs questions.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# NO `grep -q` ON THE READ END OF A PIPE. See the note in
# scripts/test-apple-angle-pin-contract.sh: with `pipefail` on it turns a live
# check into one that does not run.
pass() { printf '\033[0;32m[ok]\033[0m %s\n' "$*"; }
bad()  { printf '\033[0;31m[FAIL]\033[0m %s\n' "$*" >&2; }

run_audit() {
    audit_root="$1"
    rust="$audit_root/engine/crates/capi/src/platform/apple.rs"
    swift="$audit_root/platforms/apple/Sources/MigoAppleRenderer/MigoEngineCapabilities.swift"
    sdk="$audit_root/scripts/build-apple-sdk.sh"

    for f in "$rust" "$swift" "$sdk"; do
        [ -f "$f" ] || { printf 'VIOLATION missing-input: %s does not exist\n' "$f"; return 1; }
    done

    # Every Apple target_os the SDK builds a slice for, asked of rustc rather
    # than read out of a triple name.
    oses=""
    for platform in $(bash "$sdk" --print-platforms 2>/dev/null || true); do
        for triple in $(bash "$sdk" --print-slices "$platform" 2>/dev/null || true); do
            slice_os="$(rustc --print cfg --target "$triple" 2>/dev/null \
                | sed -n 's/^target_os="\(.*\)"$/\1/p' || true)"
            oses="$oses$triple:$slice_os,"
        done
    done

    python3 - "$rust" "$swift" "$oses" <<'PY'
import re
import sys

rust_path, swift_path, oses_raw = sys.argv[1:4]
findings = 0


def report(identifier, message):
    global findings
    print(f"VIOLATION {identifier}: {message}")
    findings += 1


engine_oses = []
for entry in oses_raw.split(","):
    if not entry.strip():
        continue
    triple, _, slice_os = entry.partition(":")
    if not slice_os:
        report("target-os-unresolved", f"rustc did not report a target_os for {triple!r}")
    elif slice_os not in engine_oses:
        engine_oses.append(slice_os)
engine_oses.sort()
if not engine_oses:
    report("platforms-unavailable", "no Apple target_os could be derived from build-apple-sdk.sh")

# ---------------------------------------------------------------- the Rust side
POSITIVE = re.compile(r'target_os\s*=\s*"([^"]+)"')
NEGATIVE = re.compile(r'not\(\s*target_os\s*=\s*"([^"]+)"\s*\)')
RUST_CONST = re.compile(r"\s*const\s+OWN_LAYER_KIND\s*:\s*u32\s*=\s*([A-Za-z0-9_:]+)\s*;")

rust_lines = open(rust_path, encoding="utf-8").read().splitlines()
rust_arms = []
for index, line in enumerate(rust_lines):
    match = RUST_CONST.fullmatch(line)
    if not match:
        continue
    # The identifier, not the value: both sides name the same MIGO_PLATFORM_*
    # constant, and comparing names is what lets this gate work without
    # evaluating either language. A numeric literal is unreadable here on
    # purpose -- it would also be unreadable to whoever maintains it.
    kind = match.group(1).rsplit("::", 1)[-1]
    if not kind.startswith("MIGO_PLATFORM_"):
        report(
            "rust-arm-unreadable",
            f"{rust_path} line {index + 1}: OWN_LAYER_KIND is {kind!r}; this gate compares the "
            "MIGO_PLATFORM_* identifier both sides name",
        )
        continue
    cursor = index - 1
    while cursor >= 0 and (
        not rust_lines[cursor].strip() or rust_lines[cursor].lstrip().startswith("//")
    ):
        cursor -= 1
    attribute = rust_lines[cursor].strip() if cursor >= 0 else ""
    gate = re.fullmatch(r"#\[cfg\((.*)\)\]", attribute)
    if not gate:
        report(
            "rust-arm-unreadable",
            f"{rust_path} line {index + 1}: OWN_LAYER_KIND is not gated by a #[cfg]",
        )
        continue
    rust_arms.append((gate.group(1).strip(), kind, index + 1))

if not rust_arms:
    report("rust-constant-missing", f"{rust_path} defines no OWN_LAYER_KIND")


def rust_selects(cfg, slice_os):
    negative = NEGATIVE.fullmatch(cfg)
    if negative:
        return negative.group(1) != slice_os
    positive = POSITIVE.fullmatch(cfg)
    if positive:
        return positive.group(1) == slice_os
    return None


for cfg, kind, line in rust_arms:
    if rust_selects(cfg, "macos") is None:
        report(
            "rust-arm-unreadable",
            f"{rust_path} line {line}: this gate reads `target_os = \"..\"` and "
            f"`not(target_os = \"..\")`, and the arm for {kind} is gated by `{cfg}`",
        )

# --------------------------------------------------------------- the Swift side
#
# Swift spells its platforms in its own case, so the names are mapped rather than
# lowercased: `os(macOS)` is rustc's `macos` and `os(iOS)` is `ios`, and no
# mechanical transformation gets both from the token.
SWIFT_OS = {
    "macOS": "macos",
    "iOS": "ios",
    "tvOS": "tvos",
    "watchOS": "watchos",
    "visionOS": "visionos",
}

swift_text = open(swift_path, encoding="utf-8").read()
body = re.search(
    r"var\s+hostSurfaceKind\s*:\s*UInt32\s*\{(.*?)\n    \}",
    swift_text,
    re.S,
)
swift_kinds = {}
if body is None:
    report(
        "swift-kind-missing",
        f"{swift_path} has no `hostSurfaceKind: UInt32` this gate can read",
    )
else:
    pending = []
    for line in body.group(1).splitlines():
        stripped = line.strip()
        branch = re.fullmatch(r"#(?:if|elseif)\s+os\(([A-Za-z]+)\)", stripped)
        if branch:
            pending.append(branch.group(1))
            continue
        returned = re.fullmatch(r"return\s+([A-Za-z0-9_.]+)", stripped)
        if returned and pending:
            token = pending.pop()
            mapped = SWIFT_OS.get(token)
            if mapped is None:
                report(
                    "swift-branch-unreadable",
                    f"{swift_path}: hostSurfaceKind names Swift platform {token!r}, which this "
                    "gate has no rustc target_os for",
                )
                continue
            kind = returned.group(1).rsplit(".", 1)[-1]
            if not kind.startswith("MIGO_PLATFORM_"):
                report(
                    "swift-branch-unreadable",
                    f"{swift_path}: the {token} branch returns {kind!r}; this gate compares the "
                    "MIGO_PLATFORM_* identifier both sides name",
                )
                continue
            swift_kinds[mapped] = kind
    if not swift_kinds:
        report(
            "swift-kind-missing",
            f"{swift_path}: hostSurfaceKind names no platform this gate could read",
        )

# ------------------------------------------------------------- the comparison
for slice_os in engine_oses:
    selected = [(cfg, kind, line) for cfg, kind, line in rust_arms if rust_selects(cfg, slice_os)]
    rust_kind = selected[0][1] if len(selected) == 1 else None
    if not selected:
        report(
            "os-only-on-one-side",
            f"the SDK builds target_os={slice_os!r} and OWN_LAYER_KIND has no arm for it",
        )
    elif len(selected) > 1:
        report(
            "rust-arm-unreadable",
            f"target_os={slice_os!r} selects {len(selected)} OWN_LAYER_KIND arms",
        )

    swift_kind = swift_kinds.get(slice_os)
    if swift_kind is None:
        report(
            "os-only-on-one-side",
            f"the SDK builds target_os={slice_os!r} and hostSurfaceKind has no branch for it, so "
            "the renderer has no surface kind on a platform it is built for",
        )

    if rust_kind is not None and swift_kind is not None and rust_kind != swift_kind:
        report(
            "kind-disagrees",
            f"target_os={slice_os!r}: the renderer presents into {swift_kind} and the C ABI "
            f"attaches {rust_kind}. preflight() would refuse a surface the library accepts",
        )

print(findings)
raise SystemExit(1 if findings else 0)
PY
}

failures=0
output="$(run_audit "$ROOT" 2>&1)" && status=0 || status=$?
if [ "$status" -eq 0 ]; then
    pass "the Apple renderer and the C ABI name the same surface kind on every platform"
else
    bad "the renderer and the C ABI disagree:"
    printf '%s\n' "$output" | sed 's/^/    /' >&2
    failures=$((failures + 1))
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/migo-apple-kind.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

RUST_REL="engine/crates/capi/src/platform/apple.rs"
SWIFT_REL="platforms/apple/Sources/MigoAppleRenderer/MigoEngineCapabilities.swift"

fixture() {
    dest="$WORK/$1"
    rm -rf "$dest"
    mkdir -p "$dest/scripts" "$dest/contracts/apple" \
             "$dest/$(dirname "$RUST_REL")" "$dest/$(dirname "$SWIFT_REL")"
    cp "$ROOT/scripts/build-apple-sdk.sh" "$dest/scripts/"
    cp "$ROOT/contracts/apple/deployment-floor.json" "$dest/contracts/apple/"
    cp "$ROOT/$RUST_REL" "$dest/$RUST_REL"
    cp "$ROOT/$SWIFT_REL" "$dest/$SWIFT_REL"
    printf '%s' "$dest"
}

expect_violation() {
    what="$1"; want_id="$2"; dest="$3"
    out="$(run_audit "$dest" 2>&1)" && rc=0 || rc=$?
    if [ "$rc" -eq 0 ]; then
        bad "injection '$what' did not turn the audit red"
        failures=$((failures + 1))
        return
    fi
    printf '%s\n' "$out" > "$WORK/last-audit.txt"
    if grep "^VIOLATION $want_id:" "$WORK/last-audit.txt" > /dev/null; then
        pass "injection '$what' -> $want_id"
    else
        bad "injection '$what' went red, but not as $want_id. What it reported:"
        printf '%s\n' "$out" | sed 's/^/    /' >&2
        failures=$((failures + 1))
    fi
}

edit() { # <fixture> <relative path> <python program over `text`>
    python3 - "$1/$2" "$3" <<'EDIT'
import pathlib, sys
path, program = pathlib.Path(sys.argv[1]), sys.argv[2]
scope = {"text": path.read_text()}
exec(program, scope)
path.write_text(scope["text"])
EDIT
}

dest="$(fixture control)"
if out="$(run_audit "$dest" 2>&1)"; then
    pass "the unmodified fixture is clean, so each injection below is the only difference"
else
    bad "the unmodified fixture is already red; no injection below proves anything:"
    printf '%s\n' "$out" | sed 's/^/    /' >&2
    failures=$((failures + 1))
fi

# The renderer starts presenting into the other Apple platform's layer. Every
# build still compiles; `preflight()` starts refusing on macOS.
dest="$(fixture swiftswap)"
edit "$dest" "$SWIFT_REL" '
text = text.replace(
    "#elseif os(macOS)\n            return MIGO_PLATFORM_MACOS_CA_METAL_LAYER",
    "#elseif os(macOS)\n            return MIGO_PLATFORM_IOS_CA_METAL_LAYER")
'
expect_violation "the renderer presents into the other platform's layer kind" \
    kind-disagrees "$dest"

# The same disagreement introduced from the Rust side instead.
dest="$(fixture rustswap)"
edit "$dest" "$RUST_REL" '
text = text.replace(
    "#[cfg(target_os = \"macos\")]\nconst OWN_LAYER_KIND: u32 = MIGO_PLATFORM_MACOS_CA_METAL_LAYER;",
    "#[cfg(target_os = \"macos\")]\nconst OWN_LAYER_KIND: u32 = MIGO_PLATFORM_IOS_CA_METAL_LAYER;")
'
expect_violation "the C ABI attaches the other platform's layer kind" kind-disagrees "$dest"

dest="$(fixture swiftdrop)"
edit "$dest" "$SWIFT_REL" '
text = text.replace(
    "        #if os(iOS)\n            return MIGO_PLATFORM_IOS_CA_METAL_LAYER\n        #elseif os(macOS)",
    "        #if os(macOS)")
'
expect_violation "the renderer loses a platform the SDK builds" os-only-on-one-side "$dest"

dest="$(fixture rustdrop)"
edit "$dest" "$RUST_REL" '
import re
text = re.sub(
    r"#\[cfg\(not\(target_os = \"macos\"\)\)\]\nconst OWN_LAYER_KIND[^\n]*\n", "", text)
'
expect_violation "the C ABI loses a platform the SDK builds" os-only-on-one-side "$dest"

dest="$(fixture swiftliteral)"
edit "$dest" "$SWIFT_REL" '
text = text.replace(
    "#elseif os(macOS)\n            return MIGO_PLATFORM_MACOS_CA_METAL_LAYER",
    "#elseif os(macOS)\n            return 5")
'
expect_violation "the renderer writes the kind as a number" swift-branch-unreadable "$dest"

dest="$(fixture rustcfg)"
edit "$dest" "$RUST_REL" '
text = text.replace(
    "#[cfg(target_os = \"macos\")]\nconst OWN_LAYER_KIND",
    "#[cfg(target_vendor = \"apple\")]\nconst OWN_LAYER_KIND")
'
expect_violation "an OWN_LAYER_KIND arm is gated on something this gate cannot evaluate" \
    rust-arm-unreadable "$dest"

dest="$(fixture noconstant)"
edit "$dest" "$RUST_REL" '
import re
text = re.sub(r"#\[cfg\((?:not\()?target_os = \"macos\"\)?\)\]\nconst OWN_LAYER_KIND[^\n]*\n", "", text)
'
expect_violation "OWN_LAYER_KIND is deleted" rust-constant-missing "$dest"

if [ "$failures" -ne 0 ]; then
    bad "$failures check(s) failed"
    exit 1
fi
echo "PASS: the renderer and the C ABI agree on every platform's surface kind, and 7 injections were each seen to break it"
