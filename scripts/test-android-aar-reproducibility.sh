#!/usr/bin/env bash
# Same-source rebuild byte equality for the one shipping archive that exists today.
#
# Item 1.12 asks for this and nothing provided it: no reproducibility or determinism gate
# existed anywhere in `scripts/`. The Android AAR is the only archive this repository
# currently produces -- the four SDK scripts populate a prefix *directory*, and item 1.11
# is what turns those into archives -- so this is where the property can be held today.
#
# Two builds under one `SOURCE_DATE_EPOCH` must produce identical bytes, and that alone is
# not evidence: an archive with no clock input at all is stable too, and so is a build that
# produced nothing. So a third build under a *different* epoch must differ. That pairing is
# this repository's rule for any absence-shaped measurement, learned when an SBOM's bytes
# were stable because the timestamp had been removed rather than made to track the epoch.
#
# What this does not claim: the native libraries' own reproducibility. The builds run with
# `--skip-rust --unverified-native-libs`, which is the documented way to exercise packaging
# logic, so what is proved here is that *packaging* the same inputs twice yields the same
# archive. The natives are covered by the V8 archive hashes in their component manifests
# and by item 1.1's from-source reproduction of both architectures.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID="$ROOT/platforms/android"
TAG="[aar-reproducibility]"

pass() { echo -e "\033[0;32m$TAG PASS $*\033[0m"; }
info() { echo -e "\033[0;36m$TAG $*\033[0m"; }
fail() { echo -e "\033[0;31m$TAG FAIL $*\033[0m" >&2; exit 1; }

PROFILE="${MIGO_AAR_REPRO_PROFILE:-full}"
ABI="${MIGO_AAR_REPRO_ABI:-arm64-v8a}"

# Probed rather than assumed, so a machine without the inputs reports that instead of
# failing as though the change under test broke something.
if [[ ! -x "$ANDROID/gradlew" ]]; then
    info "SKIP no Gradle wrapper at $ANDROID/gradlew"
    exit 0
fi
JNI_DIR="$ROOT/engine/jniLibs/$PROFILE/$ABI"
if [[ ! -f "$JNI_DIR/libmigo.so" ]]; then
    info "SKIP no staged native library at $JNI_DIR/libmigo.so"
    info "     build it with: bash scripts/build-android-so.sh $ABI"
    exit 0
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/migo-aar-repro.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

ARTIFACT="migo-$PROFILE-release-$ABI.aar"

# Each build writes into its own output directory, so no run can be compared against an
# artifact an earlier one left behind -- the stale-artifact defect this project has already
# shipped once.
build_into() { # <output-dir-name> <epoch>
    local out="$1" epoch="$2" log="$WORK/$1.log"
    mkdir -p "$ANDROID/$out"
    if ! SOURCE_DATE_EPOCH="$epoch" bash "$ROOT/scripts/build-aar.sh" \
            --build-type release --product-profile "$PROFILE" \
            --output-dir "$out" --skip-rust --unverified-native-libs "$ABI" \
            > "$log" 2>&1; then
        echo "--- last 15 lines of $log ---" >&2
        tail -15 "$log" >&2
        fail "the release AAR build failed under SOURCE_DATE_EPOCH=$epoch"
    fi
    local produced="$ANDROID/$out/$ARTIFACT"
    [[ -f "$produced" ]] || fail "no $ARTIFACT in $out after a successful build"
    cp "$produced" "$WORK/$out.aar"
    rm -rf "$ANDROID/$out"
}

EPOCH_A=1700000000
EPOCH_B=1600000000

info "building twice under SOURCE_DATE_EPOCH=$EPOCH_A"
build_into repro-a "$EPOCH_A"
build_into repro-b "$EPOCH_A"

if ! cmp -s "$WORK/repro-a.aar" "$WORK/repro-b.aar"; then
    echo "$TAG entries that differ:" >&2
    # Named per entry, because "the archives differ" does not say whether the cause is a
    # timestamp, an ordering, or a genuinely different input.
    python3 - "$WORK/repro-a.aar" "$WORK/repro-b.aar" >&2 <<'PY'
import hashlib, sys, zipfile

def digest(path):
    with zipfile.ZipFile(path) as archive:
        return {
            item.filename: hashlib.sha256(archive.read(item.filename)).hexdigest()
            for item in archive.infolist()
        }

one, two = digest(sys.argv[1]), digest(sys.argv[2])
for name in sorted(set(one) | set(two)):
    if one.get(name) != two.get(name):
        print(f"    content differs: {name}")
if set(one) != set(two):
    print(f"    entry set differs: {sorted(set(one) ^ set(two))}")
if one == two:
    print("    every entry's content matches; the difference is in the container "
          "(entry order, timestamps, or compression)")
PY
    fail "two builds of the same source produced different archives"
fi
pass "two builds under one epoch are byte-identical ($(stat -c %s "$WORK/repro-a.aar") bytes)"

info "building once under SOURCE_DATE_EPOCH=$EPOCH_B"
build_into repro-c "$EPOCH_B"
if cmp -s "$WORK/repro-a.aar" "$WORK/repro-c.aar"; then
    fail "changing SOURCE_DATE_EPOCH changed nothing, so the comparison above proves \
neither that the archive tracks the epoch nor that anything was built"
fi
pass "a different epoch produces a different archive, so the equality above is a measurement"

echo -e "\033[0;32m$TAG ok\033[0m"
