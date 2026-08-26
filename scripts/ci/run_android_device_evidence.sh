#!/usr/bin/env bash
# Install the current release AAR, run physical-device gates, and seal evidence.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

ARTIFACT=""
PACKAGE=""
SUITE_DIR=""
SOURCE_REVISION=""
DURATION=60
OUT_DIR="$REPO_ROOT/artifacts/android-release-evidence"
RENDER_MANIFEST="$REPO_ROOT/ci/render_workloads_default.json"

usage() {
    cat <<'USAGE'
Usage: run_android_device_evidence.sh --artifact FILE --package ID \
  --test-suite DIR --source-revision SHA [--duration SEC] [--out-dir DIR] \
  [--render-manifest FILE]

The external suite must provide install.sh and run.sh. run.sh must write the
requested runtime metrics, compatibility report, and complete render matrix.
USAGE
}

die() {
    echo "ERROR: $*" >&2
    exit 2
}

require_value() {
    [[ $# -ge 2 && -n "$2" ]] || die "$1 requires a value"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --artifact) require_value "$@"; ARTIFACT="$2"; shift 2 ;;
        --package) require_value "$@"; PACKAGE="$2"; shift 2 ;;
        --test-suite) require_value "$@"; SUITE_DIR="$2"; shift 2 ;;
        --source-revision) require_value "$@"; SOURCE_REVISION="$2"; shift 2 ;;
        --duration) require_value "$@"; DURATION="$2"; shift 2 ;;
        --out-dir) require_value "$@"; OUT_DIR="$2"; shift 2 ;;
        --render-manifest) require_value "$@"; RENDER_MANIFEST="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

[[ -f "$ARTIFACT" && "$ARTIFACT" == *.aar ]] || die "--artifact must name an existing AAR"
[[ -n "$PACKAGE" ]] || die "--package is required"
[[ -d "$SUITE_DIR" ]] || die "--test-suite must name an existing directory"
[[ "$SOURCE_REVISION" =~ ^[0-9a-f]{40}$ ]] || die "--source-revision must be a full lowercase Git object ID"
[[ "$DURATION" =~ ^[1-9][0-9]*$ ]] || die "--duration must be a positive integer"
[[ -f "$RENDER_MANIFEST" ]] || die "render manifest not found: $RENDER_MANIFEST"
[[ -f "$SUITE_DIR/install.sh" ]] || die "external suite install.sh is required"
[[ -f "$SUITE_DIR/run.sh" ]] || die "external suite run.sh is required"
command -v adb >/dev/null || die "adb not found in PATH"
command -v python3 >/dev/null || die "python3 not found in PATH"
command -v sha256sum >/dev/null || die "sha256sum not found in PATH"

CHECKED_OUT_REVISION="$(git -C "$REPO_ROOT" rev-parse HEAD)"
[[ "$CHECKED_OUT_REVISION" == "$SOURCE_REVISION" ]] || die "source revision does not match the checkout"
ARTIFACT_SHA256="$(sha256sum "$ARTIFACT" | awk '{print $1}')"

mapfile -t DEVICE_SERIALS < <(adb devices | awk 'NR > 1 && $2 == "device" {print $1}')
[[ ${#DEVICE_SERIALS[@]} -eq 1 ]] || die "exactly one authorized Android device is required"
DEVICE_SERIAL="${DEVICE_SERIALS[0]}"
ADB=(adb -s "$DEVICE_SERIAL")
DEVICE_ABI="$("${ADB[@]}" shell getprop ro.product.cpu.abi | tr -d '\r[:space:]')"
[[ "$DEVICE_ABI" == "arm64-v8a" ]] || die "release evidence requires a physical arm64-v8a device"
DEVICE_MODEL="$("${ADB[@]}" shell getprop ro.product.model | tr -d '\r' | xargs)"
ANDROID_API="$("${ADB[@]}" shell getprop ro.build.version.sdk | tr -d '\r[:space:]')"
[[ -n "$DEVICE_MODEL" && "$ANDROID_API" =~ ^[0-9]+$ ]] || die "device metadata is incomplete"

OUT_DIR="$(python3 -c 'import os,sys; print(os.path.abspath(sys.argv[1]))' "$OUT_DIR")"
[[ "$OUT_DIR" != "/" && "$OUT_DIR" != "$REPO_ROOT" ]] || die "unsafe evidence output directory"
INPUT_DIR="$OUT_DIR/inputs"
APK_DIR="$OUT_DIR/installed-apks"
REPORT_DIR="$OUT_DIR/reports"
RENDER_DIR="$OUT_DIR/render-matrix"
mkdir -p "$INPUT_DIR" "$APK_DIR" "$REPORT_DIR" "$RENDER_DIR"

RUNTIME_METRICS="$INPUT_DIR/runtime-metrics.json"
SUITE_REPORT="$INPUT_DIR/test-suite-report.json"
RAW_RENDER_RESULTS="$RENDER_DIR/raw-results.json"
INSTALL_RECEIPT="$REPORT_DIR/install-receipt.json"
PERF_METRICS="$REPORT_DIR/perf-metrics.json"
PERF_SUMMARY="$REPORT_DIR/perf-summary.json"
POWER_METRICS="$REPORT_DIR/power-metrics.json"
POWER_SUMMARY="$REPORT_DIR/power-summary.json"
SUITE_SUMMARY="$REPORT_DIR/test-suite-summary.json"
RENDER_SUMMARY="$REPORT_DIR/render-summary.json"
EVIDENCE="$REPORT_DIR/release-evidence.json"

# Remove only named generated outputs so a stale successful report can never
# satisfy a new run whose external suite exited without producing evidence.
rm -f "$RUNTIME_METRICS" "$SUITE_REPORT" "$RAW_RENDER_RESULTS" \
    "$INSTALL_RECEIPT" "$PERF_METRICS" "$PERF_SUMMARY" "$POWER_METRICS" \
    "$POWER_SUMMARY" "$SUITE_SUMMARY" "$RENDER_SUMMARY" "$EVIDENCE"

bash "$SUITE_DIR/install.sh" \
    --aar "$ARTIFACT" \
    --package "$PACKAGE" \
    --profile full

mapfile -t REMOTE_APKS < <(
    "${ADB[@]}" shell pm path "$PACKAGE" |
        sed -n 's/^package://p' | tr -d '\r'
)
[[ ${#REMOTE_APKS[@]} -gt 0 ]] || die "installed package exposes no APK paths"

rm -f "$APK_DIR"/installed-*.apk
RECEIPT_APK_ARGS=()
for index in "${!REMOTE_APKS[@]}"; do
    LOCAL_APK="$APK_DIR/installed-$index.apk"
    "${ADB[@]}" pull "${REMOTE_APKS[$index]}" "$LOCAL_APK" >/dev/null
    RECEIPT_APK_ARGS+=(--installed-apk "$LOCAL_APK")
done

"$SCRIPT_DIR/android_install_receipt.py" \
    --revision "$SOURCE_REVISION" \
    --artifact "$ARTIFACT" \
    --package "$PACKAGE" \
    --device-abi "$DEVICE_ABI" \
    --device-serial "$DEVICE_SERIAL" \
    "${RECEIPT_APK_ARGS[@]}" \
    --out "$INSTALL_RECEIPT"

INSTALLED_NATIVE_SHA256="$(python3 - "$INSTALL_RECEIPT" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as source:
    print(json.load(source)["installation"]["installed_native_sha256"])
PY
)"

bash "$SUITE_DIR/run.sh" \
    --package "$PACKAGE" \
    --artifact "$ARTIFACT" \
    --source-revision "$SOURCE_REVISION" \
    --artifact-sha256 "$ARTIFACT_SHA256" \
    --installed-native-sha256 "$INSTALLED_NATIVE_SHA256" \
    --device-abi "$DEVICE_ABI" \
    --profile full \
    --render-manifest "$RENDER_MANIFEST" \
    --runtime-metrics "$RUNTIME_METRICS" \
    --suite-report "$SUITE_REPORT" \
    --render-results "$RAW_RENDER_RESULTS" \
    --leave-running \
    --audio-idle

[[ -f "$RUNTIME_METRICS" ]] || die "external suite did not produce runtime metrics"
[[ -f "$SUITE_REPORT" ]] || die "external suite did not produce its compatibility report"
[[ -f "$RAW_RENDER_RESULTS" ]] || die "external suite did not produce render matrix results"

python3 "$SCRIPT_DIR/check_migo_test_suite.py" \
    --report "$SUITE_REPORT" \
    --summary-out "$SUITE_SUMMARY" \
    --source-revision "$SOURCE_REVISION" \
    --artifact-sha256 "$ARTIFACT_SHA256" \
    --installed-native-sha256 "$INSTALLED_NATIVE_SHA256" \
    --device-abi "$DEVICE_ABI" \
    --profile full \
    --package "$PACKAGE" \
    --gate

bash "$SCRIPT_DIR/collect_metrics.sh" \
    --type perf \
    --output "$PERF_METRICS" \
    --package "$PACKAGE" \
    --runtime-metrics "$RUNTIME_METRICS" \
    --artifact "$ARTIFACT" \
    --source-revision "$SOURCE_REVISION" \
    --artifact-sha256 "$ARTIFACT_SHA256" \
    --installed-native-sha256 "$INSTALLED_NATIVE_SHA256" \
    --device-abi "$DEVICE_ABI" \
    --profile full

bash "$SCRIPT_DIR/run_perf.sh" \
    --current "$PERF_METRICS" \
    --baseline "$REPO_ROOT/ci/baselines/android_perf_default.json" \
    --summary-out "$PERF_SUMMARY" \
    --fail-on-warn

bash "$SCRIPT_DIR/collect_metrics.sh" \
    --type power \
    --output "$POWER_METRICS" \
    --package "$PACKAGE" \
    --runtime-metrics "$RUNTIME_METRICS" \
    --artifact "$ARTIFACT" \
    --source-revision "$SOURCE_REVISION" \
    --artifact-sha256 "$ARTIFACT_SHA256" \
    --installed-native-sha256 "$INSTALLED_NATIVE_SHA256" \
    --device-abi "$DEVICE_ABI" \
    --profile full \
    --duration "$DURATION" \
    --interval 2

bash "$SCRIPT_DIR/run_power.sh" \
    --current "$POWER_METRICS" \
    --baseline "$REPO_ROOT/ci/baselines/android_power_default.json" \
    --summary-out "$POWER_SUMMARY" \
    --fail-on-warn

bash "$SCRIPT_DIR/run_render_matrix.sh" \
    "$RENDER_MANIFEST" "$RENDER_DIR" "$SOURCE_REVISION" "$ARTIFACT_SHA256" full
cp "$RENDER_DIR/render-compare-summary.json" "$RENDER_SUMMARY"

"$SCRIPT_DIR/release_evidence.py" create \
    --revision "$SOURCE_REVISION" \
    --artifact "$ARTIFACT" \
    --profile full \
    --device-abi "$DEVICE_ABI" \
    --installed-native-sha256 "$INSTALLED_NATIVE_SHA256" \
    --package "$PACKAGE" \
    --device-model "$DEVICE_MODEL" \
    --android-api "$ANDROID_API" \
    --device-serial "$DEVICE_SERIAL" \
    --report "perf_metrics=$PERF_METRICS" \
    --report "perf_summary=$PERF_SUMMARY" \
    --report "power_metrics=$POWER_METRICS" \
    --report "power_summary=$POWER_SUMMARY" \
    --report "render_summary=$RENDER_SUMMARY" \
    --report "suite_summary=$SUITE_SUMMARY" \
    --out "$EVIDENCE"

"$SCRIPT_DIR/release_evidence.py" verify \
    --evidence "$EVIDENCE" \
    --revision "$SOURCE_REVISION" \
    --artifact "$ARTIFACT" \
    --reports-dir "$REPORT_DIR"

echo "Android release evidence: PASS ($OUT_DIR)"
