#!/usr/bin/env bash
# The published Apple ANGLE archives must be pinned, complete, and fetched by
# hash.
#
# THE DRIFT THIS EXISTS TO CATCH is the one the ANGLE pin's own neighbours were
# built against: a dependency that is "pinned" by a URL and a hope. ANGLE
# publishes no official prebuilt binaries for any platform, so there is no
# upstream release to point at -- only bytes this project built and hosts, which
# means the only thing standing between a consumer and a substituted library is
# a committed hash that something checks before use.
#
# It also keeps the pin COMPLETE, which is the half a hash cannot do. An
# xcframework needs every slice group the engine has, and a lock file that
# covers two of three is not a broken pin, it is a pin that works perfectly for
# the platforms it mentions and silently has no opinion about the third. So the
# set of platforms is compared against scripts/build-apple-sdk.sh --print-platforms
# and the slices of each against --print-slices, rather than against a list kept
# here.
#
# Host-only: it reads the lock file and asks two scripts questions.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# NO `grep -q` ON THE READ END OF A PIPE. `set -o pipefail` is on, `grep -q`
# exits at its first match, the writer then takes SIGPIPE and exits 141, and
# pipefail reports that 141 as the pipeline's status -- so the test does not
# fail, it does not run. That is not hypothetical here: it silently disabled a
# live check in scripts/test-apple-angle-recipe-contract.sh the day the file it
# scanned grew past the point where the writer finished first.
pass() { printf '\033[0;32m[ok]\033[0m %s\n' "$*"; }
bad()  { printf '\033[0;31m[FAIL]\033[0m %s\n' "$*" >&2; }

run_audit() {
    audit_root="$1"
    lock="$audit_root/contracts/artifact-manifest/apple-angle.lock.json"
    fetch="$audit_root/scripts/fetch-apple-angle.sh"
    sdk="$audit_root/scripts/build-apple-sdk.sh"
    angle="$audit_root/scripts/build-angle-apple.sh"

    for f in "$lock" "$fetch" "$sdk" "$angle"; do
        [ -f "$f" ] || { printf 'VIOLATION missing-input: %s does not exist\n' "$f"; return 1; }
    done

    platforms="$(bash "$sdk" --print-platforms 2>/dev/null || true)"
    slices=""
    for platform in $platforms; do
        slices="$slices$platform=$(bash "$sdk" --print-slices "$platform" 2>/dev/null | tr '\n' ',');"
    done

    # What ANGLE's own loader will open on each platform, asked of the recipe
    # that owns that rule. It is what the pin's `contents` has to agree with:
    # `contents` is what a consumer unpacks, and the top of each loader path is
    # what has to be in there for the library to open at all.
    layouts=""
    for platform in $platforms; do
        layouts="$layouts$platform=$(bash "$angle" --print-loader-layout "$platform" 2>/dev/null | tr '\n' ',');"
    done

    python3 - "$lock" "$fetch" "$platforms" "$slices" "$layouts" <<'PY'
import json
import re
import sys

lock_path, fetch_path, platforms_raw, slices_raw, layouts_raw = sys.argv[1:6]
findings = 0

# "<platform>=<target> <path>,<target> <path>,;" -- one string because a shell
# cannot hand a nested mapping to a program any other way.
layouts = {}
for chunk in layouts_raw.split(";"):
    if not chunk:
        continue
    name, _, rows = chunk.partition("=")
    entries = {}
    for row in rows.split(","):
        if not row.strip():
            continue
        target, _, path = row.partition(" ")
        entries[target] = path.split("/")[0]
    layouts[name] = entries


def report(identifier, message):
    global findings
    print(f"VIOLATION {identifier}: {message}")
    findings += 1


with open(lock_path, encoding="utf-8") as handle:
    lock = json.load(handle)

platforms = platforms_raw.split()
expected_slices = {}
for entry in slices_raw.split(";"):
    if not entry:
        continue
    name, _, joined = entry.partition("=")
    expected_slices[name] = [s for s in joined.split(",") if s]

if not platforms:
    report("platforms-unavailable", "build-apple-sdk.sh --print-platforms produced nothing")

release = lock.get("release")
if not isinstance(release, str) or not release.startswith("https://github.com/"):
    report("release-not-a-github-url", f"release is {release!r}")

targets = lock.get("targets")
if not isinstance(targets, dict):
    report("no-targets", "the pin carries no artifact half; nothing can be fetched")
    targets = {}

missing = [p for p in platforms if p not in targets]
if missing:
    report(
        "platform-not-pinned",
        "the engine builds these Apple platforms and the pin does not carry them: "
        + ", ".join(missing),
    )
extra = [p for p in targets if p not in platforms]
if extra:
    report(
        "platform-not-built",
        "the pin carries platforms the engine does not build: " + ", ".join(extra),
    )

ninja_targets = lock.get("source", {}).get("ninja_targets") or []
assets = {}
for platform, target in sorted(targets.items()):
    asset = target.get("asset")
    if not isinstance(asset, str) or not asset:
        report("asset-missing", f"{platform} names no asset")
    else:
        if asset in assets:
            # GitHub release assets are one flat namespace. Two platforms
            # claiming one name is the defect windows-angle.lock.json's own note
            # records having designed around.
            report(
                "asset-name-collision",
                f"{platform} and {assets[asset]} both publish as {asset!r}",
            )
        assets[asset] = platform

    digest = target.get("sha256")
    if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
        report("sha256-malformed", f"{platform}: sha256 is {digest!r}")

    size = target.get("size_bytes")
    if not isinstance(size, int) or isinstance(size, bool) or size <= 0:
        report("size-malformed", f"{platform}: size_bytes is {size!r}")

    contents = target.get("contents")
    if not isinstance(contents, list) or not contents:
        report("contents-missing", f"{platform} declares no contents")
        contents = []
    # What has to be in `contents` is not a name pattern, it is the top of the
    # path ANGLE will open: `libEGL.framework` on the iOS family, `libEGL.dylib`
    # on macOS. Asked of scripts/build-angle-apple.sh --print-loader-layout
    # rather than matched with a prefix rule, because a prefix rule accepts
    # `libEGL.framework` on macOS -- which is exactly the repackaging that
    # breaks ANGLE's lookup of libGLESv2, and it would pin cleanly.
    wanted_entries = layouts.get(platform)
    if wanted_entries is None:
        report(
            "loader-layout-unavailable",
            f"{platform}: build-angle-apple.sh --print-loader-layout answered nothing",
        )
        wanted_entries = {}
    for ninja_target in ninja_targets:
        entry = wanted_entries.get(ninja_target)
        if entry is None:
            report(
                "loader-layout-unavailable",
                f"{platform}: the recipe has no loader path for {ninja_target!r}",
            )
            continue
        if contents.count(entry) != 1:
            report(
                "contents-incomplete",
                f"{platform}: ANGLE opens {ninja_target!r} under {entry!r}, which appears "
                f"{contents.count(entry)} times in contents {contents!r}",
            )

    pinned_slices = target.get("slices")
    want = expected_slices.get(platform)
    if want is not None and pinned_slices != want:
        report(
            "slices-disagree",
            f"{platform}: the pin says {pinned_slices!r}, build-apple-sdk.sh says {want!r}",
        )

with open(fetch_path, encoding="utf-8") as handle:
    fetch_source = "\n".join(
        line for line in handle.read().splitlines() if not line.lstrip().startswith("#")
    )

for asset in assets:
    if asset in fetch_source:
        report(
            "fetch-hardcodes-an-asset",
            f"scripts/fetch-apple-angle.sh names {asset!r}; asset names belong to the lock",
        )
if re.search(r"[0-9a-f]{64}", fetch_source):
    report(
        "fetch-hardcodes-a-hash",
        "scripts/fetch-apple-angle.sh contains a 64-character hash outside a comment",
    )

print(findings)
raise SystemExit(1 if findings else 0)
PY
}

failures=0
output="$(run_audit "$ROOT" 2>&1)" && status=0 || status=$?
if [ "$status" -eq 0 ]; then
    pass "the Apple ANGLE pin is complete, hashed, and the fetch script derives everything from it"
else
    bad "the pin violates the contract:"
    printf '%s\n' "$output" | sed 's/^/    /' >&2
    failures=$((failures + 1))
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/migo-angle-pin.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

fixture() {
    dest="$WORK/$1"
    rm -rf "$dest"
    mkdir -p "$dest/scripts" "$dest/contracts/artifact-manifest" "$dest/contracts/apple"
    cp "$ROOT/scripts/fetch-apple-angle.sh" "$dest/scripts/"
    cp "$ROOT/scripts/build-apple-sdk.sh" "$dest/scripts/"
    cp "$ROOT/scripts/build-angle-apple.sh" "$dest/scripts/"
    cp "$ROOT/contracts/artifact-manifest/apple-angle.lock.json" "$dest/contracts/artifact-manifest/"
    cp "$ROOT/contracts/apple/deployment-floor.json" "$dest/contracts/apple/"
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
    if grep -q "^VIOLATION $want_id:" "$WORK/last-audit.txt"; then
        pass "injection '$what' -> $want_id"
    else
        bad "injection '$what' went red, but not as $want_id. What it reported:"
        printf '%s\n' "$out" | sed 's/^/    /' >&2
        failures=$((failures + 1))
    fi
}

edit_lock() {
    python3 - "$1" "$2" <<'EDIT'
import json, pathlib, sys
path, program = pathlib.Path(sys.argv[1]), sys.argv[2]
doc = json.loads(path.read_text())
exec(program, {"doc": doc})
path.write_text(json.dumps(doc, indent=2) + "\n")
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

dest="$(fixture dropped)"
edit_lock "$dest/contracts/artifact-manifest/apple-angle.lock.json" 'doc["targets"].pop("macos")'
expect_violation "a platform loses its pin" platform-not-pinned "$dest"

dest="$(fixture badhash)"
edit_lock "$dest/contracts/artifact-manifest/apple-angle.lock.json" 'doc["targets"]["ios"]["sha256"] = "not-a-hash"'
expect_violation "a hash stops being a hash" sha256-malformed "$dest"

dest="$(fixture collide)"
edit_lock "$dest/contracts/artifact-manifest/apple-angle.lock.json" 'doc["targets"]["macos"]["asset"] = doc["targets"]["ios"]["asset"]'
expect_violation "two platforms publish under one asset name" asset-name-collision "$dest"

dest="$(fixture halfpack)"
edit_lock "$dest/contracts/artifact-manifest/apple-angle.lock.json" 'doc["targets"]["ios"]["contents"] = doc["targets"]["ios"]["contents"][:1]'
expect_violation "an archive ships one of the two libraries" contents-incomplete "$dest"

# The one a prefix rule would have accepted: macOS pinned as holding framework
# BUNDLES. Every name still starts with the ninja target, the archive would
# verify, and ANGLE would then look for libGLESv2.dylib beside a binary that is
# two directories further down and never find it.
dest="$(fixture wrappedmac)"
edit_lock "$dest/contracts/artifact-manifest/apple-angle.lock.json" \
    'doc["targets"]["macos"]["contents"] = ["libEGL.framework", "libGLESv2.framework"]'
expect_violation "macOS is pinned as holding framework bundles" contents-incomplete "$dest"

dest="$(fixture sliceskew)"
edit_lock "$dest/contracts/artifact-manifest/apple-angle.lock.json" 'doc["targets"]["macos"]["slices"] = ["aarch64-apple-darwin"]'
expect_violation "the pin drops a slice the engine builds" slices-disagree "$dest"

dest="$(fixture size)"
edit_lock "$dest/contracts/artifact-manifest/apple-angle.lock.json" 'doc["targets"]["ios"]["size_bytes"] = 0'
expect_violation "a size stops being a size" size-malformed "$dest"

dest="$(fixture nourl)"
edit_lock "$dest/contracts/artifact-manifest/apple-angle.lock.json" 'doc["release"] = "http://mirror.example/angle"'
expect_violation "the release moves off a hashed, owned location" release-not-a-github-url "$dest"

dest="$(fixture hardcoded)"
python3 - "$dest" <<'HARD'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
lock = json.loads((root / "contracts/artifact-manifest/apple-angle.lock.json").read_text())
asset = lock["targets"]["ios"]["asset"]
script = root / "scripts/fetch-apple-angle.sh"
text = script.read_text()
script.write_text(text.replace("failures=0\n", f'FALLBACK_ASSET="{asset}"\nfailures=0\n', 1))
HARD
expect_violation "the fetch script names an asset itself" fetch-hardcodes-an-asset "$dest"

if [ "$failures" -ne 0 ]; then
    bad "$failures check(s) failed"
    exit 1
fi
echo "PASS: the Apple ANGLE pin contract holds, and 9 injections were each seen to break it"
