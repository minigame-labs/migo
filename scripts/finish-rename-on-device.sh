#!/usr/bin/env bash
# Finish the 2026-07-21 crate rename on a connected aarch64 device.
#
# The rename changed `engine/Cargo.lock`, which `scripts/lib/snapshot-fingerprint.sh`
# hashes, so every committed aarch64 snapshot went stale the moment the packages
# were renamed. V8 snapshots are architecture-bound and the generator runs on the
# target ABI, so a host cannot produce them -- this is the one step of the
# refactor that requires hardware.
#
# Usage:
#   bash scripts/finish-rename-on-device.sh [--serial SERIAL]
#
# Prerequisite on Windows (WSL2 does not see USB by default):
#   usbipd list
#   usbipd bind   --busid <phone busid>
#   usbipd attach --wsl --busid <phone busid>

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SERIAL="${ANDROID_SERIAL:-}"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --serial) SERIAL="$2"; shift 2 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

ADB="${ADB:-$HOME/Android/Sdk/platform-tools/adb}"
[[ -x "$ADB" ]] || ADB="$(command -v adb || true)"
[[ -n "$ADB" && -x "$ADB" ]] || { echo "adb not found (set ADB=)" >&2; exit 1; }

step() { printf '\n\033[0;36m[%s]\033[0m %s\n' "$1" "$2"; }
fail() { printf '\033[0;31mFAIL\033[0m %s\n' "$1" >&2; exit 1; }

step 1/6 "checking the device is visible"
if [[ -z "$SERIAL" ]]; then
    mapfile -t devices < <("$ADB" devices | awk 'NR>1 && $2=="device" {print $1}')
    [[ ${#devices[@]} -eq 1 ]] || fail "expected exactly one device, found ${#devices[@]}. Pass --serial."
    SERIAL="${devices[0]}"
fi
echo "using device $SERIAL"

# The device sleeps into zero frames and zero telemetry, so pin it awake before
# anything is measured.
"$ADB" -s "$SERIAL" shell svc power stayon true >/dev/null 2>&1 || true
"$ADB" -s "$SERIAL" shell input keyevent KEYCODE_WAKEUP >/dev/null 2>&1 || true

step 2/6 "regenerating the three aarch64 snapshots"
for spec in "host full" "host slim" "worker full"; do
    read -r kind profile <<<"$spec"
    echo "--- kind=$kind profile=$profile ---"
    ANDROID_SERIAL="$SERIAL" bash scripts/gen-snapshot.sh arm64 \
        --product-profile "$profile" --snapshot-kind "$kind" \
        || fail "snapshot generation failed for $kind/$profile"
done

step 3/6 "asserting freshness (all three aarch64 kind/profile sets must be clean)"
# The checker takes architectures as positional arguments, and it is asked for
# aarch64 only on purpose: an absent x86_64 snapshot is reported as STALE and
# exits 1, so a bare invocation always fails here regardless of what this device
# just produced. The missing x86_64 set is a separate, pre-existing gap
# (architecture docs section 6.4.3) that a phone cannot close -- x86_64
# snapshots need an emulator. Asserting per-architecture keeps the exit code
# meaningful instead of grepping output for a line that is expected to be there.
for spec in "host full" "host slim" "worker full"; do
    read -r kind profile <<<"$spec"
    bash scripts/check-snapshot-freshness.sh \
        --snapshot-kind "$kind" --product-profile "$profile" aarch64 \
        || fail "aarch64 snapshot not fresh for $kind/$profile"
done

step 4/6 "building the debug AAR (release is gated on the manifest chain; debug is the correctness lane)"
bash scripts/build-aar.sh debug arm64-v8a || fail "debug AAR build failed"

step 5/6 "host + contract regression sweep"
bash scripts/dev-test-host.sh test -p migo-shared -p migo-io -p migo-runtime-v8 --lib \
    || fail "host tests failed"
for g in test-core-v8-boundary-contract test-platform-v8-boundary-contract \
         test-platform-services-capability-contract test-surface-attachment-contract \
         test-c-abi-surface-candidate test-capi-platform-contract \
         test-r9-worker-snapshot test-product-profiles \
         test-example-content-namespace-contract; do
    printf '%-46s ' "$g"
    if bash "scripts/$g.sh" >/tmp/finish-rename-$g.log 2>&1; then echo PASS; else echo FAIL; fail "$g"; fi
done

step 6/6 "done"
cat <<'EOF'

Snapshots are fresh again and the debug AAR built. What still needs a human at
the device, because it cannot be asserted from a script:

  * install the AAR into the bench shell and confirm rendering, the
    background-and-return path (the Canvas2D state-carry fix), and clean
    lifecycle;
  * run examples/c-host/android to re-check the pure-native host after the
    rename (build with scripts/build-android-c-host.sh);
  * only then is the rename branch mergeable, because a release AAR needs the
    snapshot chain this script restored.
EOF
