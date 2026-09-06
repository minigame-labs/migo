#!/usr/bin/env bash
# The presenter's ANGLE library name must be the name the recipe says ANGLE's own
# loader will open.
#
# THE DRIFT THIS EXISTS TO CATCH is a rule written down twice with nothing tying
# the copies together. `engine/crates/platform/src/apple/presenter.rs` picks the
# file it falls back to `dlopen`ing per Apple platform; `scripts/build-angle-apple.sh
# --print-loader-layout` decides, for the same platform, what the build produces
# and therefore what ANGLE itself will look for. They agree today. Nothing makes
# them keep agreeing, and every way of breaking them apart compiles:
#
#   - The macOS arm is the one at risk, and the repository has already reasoned
#     its way to the wrong answer once. `xcodebuild -create-xcframework` refuses
#     to hold framework bundles and plain libraries in one bundle, and the tidy
#     answer -- wrap the macOS libraries in frameworks so all three platforms
#     match -- was ruled correct on the fifth ANGLE run and is wrong: it moves
#     libEGL into Versions/A/, ANGLE then searches Versions/A/libGLESv2.dylib,
#     and nothing finds it. A presenter that had been "helpfully" updated to
#     match such a repackaging would name libEGL.framework/libEGL on macOS, and
#     no build step anywhere would go red.
#   - The failure lands at `eglGetDisplay` on a real device with nothing from
#     this repository on the stack to say why. That is the most expensive place
#     this project has to learn anything, which is what makes a host-only gate
#     worth its lines.
#
# WHY `rustc --print cfg` AND NOT A TRIPLE NAME PATTERN. The presenter selects
# its arms on `target_os`, and the recipe groups slices into ios / ios-simulator /
# macos. Tying those together needs to know which `target_os` each slice has, and
# the toolchain is the only thing entitled to answer that: reading "darwin" or
# "ios" out of the triple is a naming rule that would quietly mis-group the first
# Apple platform whose triple does not follow it. So the platform list comes from
# build-apple-sdk.sh, the slices from build-apple-sdk.sh, and the target_os of
# each slice from rustc.
#
# Host-only: it reads one source file and asks three programs questions.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# NO `grep -q` ON THE READ END OF A PIPE anywhere in this file. With `pipefail`
# on, `grep -q` exits at its first match, the writer takes SIGPIPE, and the
# pipeline reports 141 -- so the check does not fail, it stops running. That
# silently disabled a live check in scripts/test-apple-angle-recipe-contract.sh.
pass() { printf '\033[0;32m[ok]\033[0m %s\n' "$*"; }
bad()  { printf '\033[0;31m[FAIL]\033[0m %s\n' "$*" >&2; }

run_audit() {
    audit_root="$1"
    presenter="$audit_root/engine/crates/platform/src/apple/presenter.rs"
    angle="$audit_root/scripts/build-angle-apple.sh"
    sdk="$audit_root/scripts/build-apple-sdk.sh"
    lock="$audit_root/contracts/artifact-manifest/apple-angle.lock.json"

    for f in "$presenter" "$angle" "$sdk" "$lock"; do
        [ -f "$f" ] || { printf 'VIOLATION missing-input: %s does not exist\n' "$f"; return 1; }
    done

    platforms="$(bash "$sdk" --print-platforms 2>/dev/null || true)"

    # "<platform>=<target> <path>,...;" and "<platform>=<target_os>,...;" -- one
    # string each, because a shell cannot hand a nested mapping to a program any
    # other way. The same shape scripts/test-apple-angle-pin-contract.sh uses.
    layouts=""
    families=""
    for platform in $platforms; do
        layouts="$layouts$platform=$(bash "$angle" --print-loader-layout "$platform" 2>/dev/null | tr '\n' ',');"
        slice_oses=""
        for triple in $(bash "$sdk" --print-slices "$platform" 2>/dev/null || true); do
            # `rustc --print cfg` answers for any target it knows, installed or
            # not, so this needs no Apple toolchain and no added rustup targets.
            slice_os="$(rustc --print cfg --target "$triple" 2>/dev/null \
                | sed -n 's/^target_os="\(.*\)"$/\1/p' || true)"
            slice_oses="$slice_oses$triple:$slice_os,"
        done
        families="$families$platform=$slice_oses;"
    done

    python3 - "$presenter" "$lock" "$platforms" "$layouts" "$families" <<'PY'
import json
import re
import sys

presenter_path, lock_path, platforms_raw, layouts_raw, families_raw = sys.argv[1:6]
findings = 0


def report(identifier, message):
    global findings
    print(f"VIOLATION {identifier}: {message}")
    findings += 1


# The ninja target that builds the EGL entry point. Written here because nothing
# machine-readable links "the library a presenter dlopens for eglGetProcAddress"
# to a build target name -- but asserted against the lock below, so an upstream
# rename fails this gate closed instead of letting it check nothing.
EGL_NINJA_TARGET = "libEGL"


def parse_map(raw):
    out = {}
    for chunk in raw.split(";"):
        if not chunk:
            continue
        name, _, rows = chunk.partition("=")
        out[name] = [r for r in rows.split(",") if r.strip()]
    return out


platforms = platforms_raw.split()
if not platforms:
    report("platforms-unavailable", "build-apple-sdk.sh --print-platforms produced nothing")

with open(lock_path, encoding="utf-8") as handle:
    lock = json.load(handle)

ninja_targets = lock.get("source", {}).get("ninja_targets") or []
if EGL_NINJA_TARGET not in ninja_targets:
    report(
        "egl-target-not-built",
        f"the pin builds {ninja_targets!r}, which does not include {EGL_NINJA_TARGET!r}; "
        "the presenter opens EGL by name and nothing here knows what to compare against",
    )

layouts = parse_map(layouts_raw)
families = parse_map(families_raw)

# target_os -> {loader path: [the platforms that reported it]}
expected = {}
for platform in platforms:
    rows = layouts.get(platform)
    if not rows:
        report(
            "loader-layout-unavailable",
            f"{platform}: build-angle-apple.sh --print-loader-layout answered nothing",
        )
        continue
    paths = {}
    for row in rows:
        target, _, path = row.partition(" ")
        paths[target] = path
    egl_path = paths.get(EGL_NINJA_TARGET)
    if egl_path is None:
        report(
            "loader-layout-unavailable",
            f"{platform}: the recipe has no loader path for {EGL_NINJA_TARGET!r}",
        )
        continue

    slices = families.get(platform) or []
    if not slices:
        report("slices-unavailable", f"{platform}: build-apple-sdk.sh --print-slices answered nothing")
    for entry in slices:
        triple, _, slice_os = entry.partition(":")
        if not slice_os:
            report(
                "target-os-unresolved",
                f"rustc did not report a target_os for {triple!r}",
            )
            continue
        expected.setdefault(slice_os, {}).setdefault(egl_path, []).append(platform)

# One constant serves a whole target_os. Two platforms that share one and
# disagree about the path cannot both be right, and the presenter has no way to
# tell them apart.
for slice_os, paths in sorted(expected.items()):
    if len(paths) > 1:
        report(
            "platform-family-disagrees",
            f"target_os={slice_os!r} is built by platforms that want different ANGLE layouts: "
            + "; ".join(f"{path!r} for {sorted(set(who))}" for path, who in sorted(paths.items())),
        )

with open(presenter_path, encoding="utf-8") as handle:
    lines = handle.read().splitlines()

CONST = re.compile(r'\s*const\s+APPLE_EGL_LIBRARY\s*:\s*&str\s*=\s*"([^"]*)"\s*;')
POSITIVE = re.compile(r'target_os\s*=\s*"([^"]+)"')
NEGATIVE = re.compile(r'not\(\s*target_os\s*=\s*"([^"]+)"\s*\)')

arms = []
for index, line in enumerate(lines):
    match = CONST.fullmatch(line)
    if not match:
        continue
    value = match.group(1)
    # Walk up past blank lines and doc comments to the attribute that gates it.
    cursor = index - 1
    while cursor >= 0 and (not lines[cursor].strip() or lines[cursor].lstrip().startswith("//")):
        cursor -= 1
    attribute = lines[cursor].strip() if cursor >= 0 else ""
    gate = re.fullmatch(r"#\[cfg\((.*)\)\]", attribute)
    if not gate:
        report(
            "arm-has-no-cfg",
            f"line {index + 1}: APPLE_EGL_LIBRARY = {value!r} is not gated by a #[cfg]; "
            "one name cannot be right for both Apple product shapes",
        )
        continue
    arms.append((gate.group(1).strip(), value, index + 1))

if not arms:
    report(
        "presenter-constant-missing",
        f"{presenter_path} defines no APPLE_EGL_LIBRARY; there is nothing to hold to the recipe",
    )


def selects(cfg, slice_os):
    """Whether a build for `slice_os` compiles this arm.

    Deliberately total over two shapes only. A cfg this cannot read is reported
    rather than guessed at: the alternative is a gate that silently stops
    checking the arm someone just rewrote.
    """
    negative = NEGATIVE.fullmatch(cfg)
    if negative:
        return negative.group(1) != slice_os
    positive = POSITIVE.fullmatch(cfg)
    if positive:
        return positive.group(1) == slice_os
    return None


for cfg, value, line in arms:
    if selects(cfg, "macos") is None:
        report(
            "cfg-shape-unrecognised",
            f"line {line}: this gate reads `target_os = \"..\"` and `not(target_os = \"..\")`, "
            f"and the arm for {value!r} is gated by `{cfg}`",
        )

readable = [(cfg, value, line) for cfg, value, line in arms if selects(cfg, "macos") is not None]

for slice_os, paths in sorted(expected.items()):
    if len(paths) != 1:
        continue  # already reported as platform-family-disagrees
    want = next(iter(paths))
    selected = [(cfg, value, line) for cfg, value, line in readable if selects(cfg, slice_os)]
    if not selected:
        report(
            "no-arm-selected",
            f"a target_os={slice_os!r} build selects no APPLE_EGL_LIBRARY arm, and the engine "
            f"builds that target_os",
        )
        continue
    if len(selected) > 1:
        report(
            "overlapping-arms",
            f"target_os={slice_os!r} selects {len(selected)} arms (lines "
            + ", ".join(str(line) for _, _, line in selected)
            + ")",
        )
        continue
    cfg, value, line = selected[0]
    if value != want:
        report(
            "arm-disagrees-with-recipe",
            f"target_os={slice_os!r}: the presenter opens {value!r} (line {line}, `{cfg}`) and "
            f"the recipe says ANGLE will be at {want!r}",
        )

# An arm no Apple platform the engine builds ever selects is a rule nobody
# reads, and it reads as coverage.
engine_oses = sorted(expected)
for cfg, value, line in readable:
    if not any(selects(cfg, slice_os) for slice_os in engine_oses):
        report(
            "arm-unreachable",
            f"line {line}: `{cfg}` selects {value!r} for no target_os the engine builds "
            f"({engine_oses})",
        )

print(findings)
raise SystemExit(1 if findings else 0)
PY
}

failures=0
output="$(run_audit "$ROOT" 2>&1)" && status=0 || status=$?
if [ "$status" -eq 0 ]; then
    pass "the Apple presenter opens the ANGLE library the recipe says it will find"
else
    bad "the presenter and the recipe disagree:"
    printf '%s\n' "$output" | sed 's/^/    /' >&2
    failures=$((failures + 1))
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/migo-apple-egl-name.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

fixture() {
    dest="$WORK/$1"
    rm -rf "$dest"
    mkdir -p "$dest/scripts" "$dest/contracts/artifact-manifest" "$dest/contracts/apple" \
             "$dest/engine/crates/platform/src/apple"
    cp "$ROOT/scripts/build-angle-apple.sh" "$dest/scripts/"
    cp "$ROOT/scripts/build-apple-sdk.sh" "$dest/scripts/"
    cp "$ROOT/contracts/artifact-manifest/apple-angle.lock.json" "$dest/contracts/artifact-manifest/"
    cp "$ROOT/contracts/apple/deployment-floor.json" "$dest/contracts/apple/"
    cp "$ROOT/engine/crates/platform/src/apple/presenter.rs" \
       "$dest/engine/crates/platform/src/apple/"
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

edit_presenter() {
    python3 - "$1/engine/crates/platform/src/apple/presenter.rs" "$2" <<'EDIT'
import pathlib, sys
path, program = pathlib.Path(sys.argv[1]), sys.argv[2]
text = path.read_text()
scope = {"text": text}
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

# The exact mistake the fifth ANGLE run's ruling would have produced: the macOS
# presenter taught to look inside a framework bundle. Nothing in a build would
# notice.
dest="$(fixture wrappedmac)"
edit_presenter "$dest" 'text = text.replace("\"libEGL.dylib\"", "\"libEGL.framework/libEGL\"")'
expect_violation "the macOS arm looks inside a framework bundle" arm-disagrees-with-recipe "$dest"

dest="$(fixture flatios)"
edit_presenter "$dest" 'text = text.replace("\"libEGL.framework/libEGL\"", "\"libEGL.dylib\"")'
expect_violation "the iOS arm looks for a bare dylib" arm-disagrees-with-recipe "$dest"

# The other direction, and the one that proves this gate reads the recipe rather
# than comparing the presenter with itself: the RECIPE changes and the presenter
# does not follow.
dest="$(fixture recipemoved)"
python3 - "$dest/scripts/build-angle-apple.sh" <<'RECIPE'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
before = "mac) printf '%s.dylib' \"$2\" ;;"
assert text.count(before) == 1, text.count(before)
path.write_text(text.replace(before, "mac) printf 'lib/%s.dylib' \"$2\" ;;"))
RECIPE
expect_violation "the recipe moves the macOS product and the presenter does not follow" \
    arm-disagrees-with-recipe "$dest"

dest="$(fixture noconstant)"
edit_presenter "$dest" '
import re
text = re.sub(r"#\[cfg\((?:not\()?target_os = \"macos\"\)?\)\]\nconst APPLE_EGL_LIBRARY[^\n]*\n", "", text)
'
expect_violation "the constant is deleted" presenter-constant-missing "$dest"

dest="$(fixture ungated)"
edit_presenter "$dest" '
import re
text = re.sub(r"#\[cfg\(not\(target_os = \"macos\"\)\)\]\nconst APPLE_EGL_LIBRARY[^\n]*\n", "", text)
text = text.replace("#[cfg(target_os = \"macos\")]\nconst APPLE_EGL_LIBRARY", "const APPLE_EGL_LIBRARY")
'
expect_violation "one ungated constant serves both product shapes" arm-has-no-cfg "$dest"

dest="$(fixture othercfg)"
edit_presenter "$dest" '
text = text.replace("#[cfg(target_os = \"macos\")]\nconst APPLE_EGL_LIBRARY",
                    "#[cfg(target_vendor = \"apple\")]\nconst APPLE_EGL_LIBRARY")
'
expect_violation "an arm is gated on something this gate cannot evaluate" \
    cfg-shape-unrecognised "$dest"

dest="$(fixture nomacarm)"
edit_presenter "$dest" '
import re
text = re.sub(r"#\[cfg\(target_os = \"macos\"\)\]\nconst APPLE_EGL_LIBRARY[^\n]*\n", "", text)
'
expect_violation "the macOS arm is removed while macOS is still built" no-arm-selected "$dest"

dest="$(fixture deadarm)"
edit_presenter "$dest" '
text = text.replace("#[cfg(target_os = \"macos\")]\nconst APPLE_EGL_LIBRARY",
                    "#[cfg(target_os = \"tvos\")]\nconst APPLE_EGL_LIBRARY_TVOS: &str = \"libEGL.dylib\";\n#[cfg(target_os = \"macos\")]\nconst APPLE_EGL_LIBRARY")
text = text.replace("APPLE_EGL_LIBRARY_TVOS", "APPLE_EGL_LIBRARY")
'
expect_violation "an arm is added for a platform the SDK does not build" arm-unreachable "$dest"

# One target_os, two platforms, two answers: the iOS family splits and the
# presenter's single non-macOS arm can no longer be right for both.
dest="$(fixture familysplit)"
python3 - "$dest/scripts/build-angle-apple.sh" <<'SPLIT'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
before = """loader_layout_for_family() {
    for layout_target in $NINJA_TARGETS; do"""
after = """loader_layout_for_family() {
    if [ "$1" = ios ] && [ "${PLATFORM:-}" = ios-simulator ]; then
        for layout_target in $NINJA_TARGETS; do
            printf '%s %s.dylib\\n' "$layout_target" "$layout_target"
        done
        return 0
    fi
    for layout_target in $NINJA_TARGETS; do"""
assert text.count(before) == 1, text.count(before)
path.write_text(text.replace(before, after))
SPLIT
expect_violation "one target_os is built by platforms wanting different layouts" \
    platform-family-disagrees "$dest"

dest="$(fixture noegltarget)"
python3 - "$dest/contracts/artifact-manifest/apple-angle.lock.json" <<'LOCK'
import json, pathlib, sys
path = pathlib.Path(sys.argv[1])
doc = json.loads(path.read_text())
doc["source"]["ninja_targets"] = [t for t in doc["source"]["ninja_targets"] if t != "libEGL"]
path.write_text(json.dumps(doc, indent=2) + "\n")
LOCK
expect_violation "the pin stops building the EGL library the presenter opens" \
    egl-target-not-built "$dest"

if [ "$failures" -ne 0 ]; then
    bad "$failures check(s) failed"
    exit 1
fi
echo "PASS: the presenter's ANGLE library name is the recipe's, and 10 injections were each seen to break it"
