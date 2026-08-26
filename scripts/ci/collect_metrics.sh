#!/usr/bin/env bash
# Collect artifact-bound metrics from one Android device.
#
# Runtime metrics come from the external conformance workload. This script
# independently verifies their source/artifact/install bindings and augments
# them with process, battery, thermal, memory, and package-size data.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

METRIC_TYPE="all"
OUTPUT="ci/metrics/current_metrics.json"
PACKAGE=""
RUNTIME_METRICS=""
ARTIFACT=""
SOURCE_REVISION=""
ARTIFACT_SHA256=""
INSTALLED_NATIVE_SHA256=""
DEVICE_ABI=""
PROFILE=""
DURATION=30
INTERVAL=2
SESSION_ID=0

usage() {
    cat <<'USAGE'
Usage: collect_metrics.sh --package ID --runtime-metrics FILE --artifact FILE \
  --source-revision SHA --artifact-sha256 SHA256 \
  --installed-native-sha256 SHA256 --device-abi ABI --profile full \
  [--type perf|power|all] [--output FILE] [--duration SEC] [--interval SEC] \
  [--session-id ID]
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
        --type) require_value "$@"; METRIC_TYPE="$2"; shift 2 ;;
        --output) require_value "$@"; OUTPUT="$2"; shift 2 ;;
        --package) require_value "$@"; PACKAGE="$2"; shift 2 ;;
        --runtime-metrics) require_value "$@"; RUNTIME_METRICS="$2"; shift 2 ;;
        --artifact) require_value "$@"; ARTIFACT="$2"; shift 2 ;;
        --source-revision) require_value "$@"; SOURCE_REVISION="$2"; shift 2 ;;
        --artifact-sha256) require_value "$@"; ARTIFACT_SHA256="$2"; shift 2 ;;
        --installed-native-sha256)
            require_value "$@"; INSTALLED_NATIVE_SHA256="$2"; shift 2 ;;
        --device-abi) require_value "$@"; DEVICE_ABI="$2"; shift 2 ;;
        --profile) require_value "$@"; PROFILE="$2"; shift 2 ;;
        --duration) require_value "$@"; DURATION="$2"; shift 2 ;;
        --interval) require_value "$@"; INTERVAL="$2"; shift 2 ;;
        --session-id) require_value "$@"; SESSION_ID="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

[[ -n "$PACKAGE" ]] || die "--package is required"
[[ -n "$RUNTIME_METRICS" ]] || die "--runtime-metrics is required"
[[ -n "$ARTIFACT" ]] || die "--artifact is required"
[[ -n "$SOURCE_REVISION" ]] || die "--source-revision is required"
[[ -n "$ARTIFACT_SHA256" ]] || die "--artifact-sha256 is required"
[[ -n "$INSTALLED_NATIVE_SHA256" ]] || die "--installed-native-sha256 is required"
[[ -n "$DEVICE_ABI" ]] || die "--device-abi is required"
[[ -n "$PROFILE" ]] || die "--profile is required"
[[ "$METRIC_TYPE" =~ ^(perf|power|all)$ ]] || die "--type must be perf, power, or all"
[[ "$DURATION" =~ ^[1-9][0-9]*$ ]] || die "--duration must be a positive integer"
[[ "$INTERVAL" =~ ^[1-9][0-9]*$ ]] || die "--interval must be a positive integer"
[[ "$SESSION_ID" =~ ^[0-9]+$ ]] || die "--session-id must be a non-negative integer"
[[ "$SOURCE_REVISION" =~ ^[0-9a-f]{40}$ ]] || die "source revision must be a full lowercase Git object ID"
[[ "$ARTIFACT_SHA256" =~ ^[0-9a-f]{64}$ ]] || die "artifact SHA-256 is malformed"
[[ "$INSTALLED_NATIVE_SHA256" =~ ^[0-9a-f]{64}$ ]] || die "installed native SHA-256 is malformed"
[[ "$DEVICE_ABI" =~ ^(arm64-v8a|x86_64)$ ]] || die "unsupported device ABI: $DEVICE_ABI"
[[ "$PROFILE" == "full" ]] || die "release metrics must exercise the full profile"
[[ -f "$RUNTIME_METRICS" ]] || die "runtime metrics file not found: $RUNTIME_METRICS"
[[ -f "$ARTIFACT" && "$ARTIFACT" == *.aar ]] || die "artifact must be an existing AAR"
command -v python3 >/dev/null || die "python3 not found in PATH"
command -v adb >/dev/null || die "adb not found in PATH"
command -v sha256sum >/dev/null || die "sha256sum not found in PATH"

ACTUAL_ARTIFACT_SHA256="$(sha256sum "$ARTIFACT" | awk '{print $1}')"
[[ "$ACTUAL_ARTIFACT_SHA256" == "$ARTIFACT_SHA256" ]] || die "artifact SHA-256 does not match the AAR"

# Verify both the device slice and the baseline's named arm64 slice directly
# from the AAR. A full release AAR must contain both when testing x86 hardware.
AAR_INFO="$(python3 - "$ARTIFACT" "$DEVICE_ABI" <<'PY'
import hashlib
import sys
import zipfile

artifact, abi = sys.argv[1:]
try:
    with zipfile.ZipFile(artifact) as archive:
        selected = archive.read(f"jni/{abi}/libmigo.so")
        arm64 = archive.read("jni/arm64-v8a/libmigo.so")
except (OSError, zipfile.BadZipFile, KeyError) as error:
    raise SystemExit(f"invalid full AAR: {error}")
print(hashlib.sha256(selected).hexdigest(), f"{len(arm64) / 1048576:.6f}")
PY
)" || die "could not inspect native slices in AAR"
read -r ARTIFACT_NATIVE_SHA256 SO_ARM64_SIZE_MB <<< "$AAR_INFO"
[[ "$ARTIFACT_NATIVE_SHA256" == "$INSTALLED_NATIVE_SHA256" ]] || \
    die "installed native SHA-256 does not match the AAR device slice"

mapfile -t DEVICE_SERIALS < <(adb devices | awk 'NR > 1 && $2 == "device" {print $1}')
[[ ${#DEVICE_SERIALS[@]} -eq 1 ]] || die "exactly one authorized Android device is required"
DEVICE_SERIAL="${DEVICE_SERIALS[0]}"
ADB=(adb -s "$DEVICE_SERIAL")

DEVICE_ABI_ACTUAL="$("${ADB[@]}" shell getprop ro.product.cpu.abi | tr -d '\r[:space:]')"
[[ "$DEVICE_ABI_ACTUAL" == "$DEVICE_ABI" ]] || die "declared device ABI does not match connected device"
"${ADB[@]}" shell pm path "$PACKAGE" 2>/dev/null | grep -q '^package:' || \
    die "package is not installed: $PACKAGE"

get_pid() {
    "${ADB[@]}" shell pidof "$PACKAGE" 2>/dev/null | tr -d '\r' | xargs
}

PID="$(get_pid)"
[[ "$PID" =~ ^[0-9]+$ ]] || die "package must have exactly one running process"
INITIAL_PID="$PID"

DEVICE_MODEL="$("${ADB[@]}" shell getprop ro.product.model | tr -d '\r' | xargs)"
ANDROID_API="$("${ADB[@]}" shell getprop ro.build.version.sdk | tr -d '\r[:space:]')"
[[ -n "$DEVICE_MODEL" ]] || die "device model is unavailable"
[[ "$ANDROID_API" =~ ^[0-9]+$ ]] || die "Android API level is unavailable"

# The workload report must attest the exact source, bytes, installation, ABI,
# profile, and package that this collector was asked to measure.
RUNTIME_INFO="$(python3 - "$RUNTIME_METRICS" "$SOURCE_REVISION" "$ARTIFACT_SHA256" \
    "$INSTALLED_NATIVE_SHA256" "$DEVICE_ABI" "$PROFILE" "$PACKAGE" <<'PY'
import json
import sys

path, revision, artifact_hash, installed_hash, abi, profile, package = sys.argv[1:]
try:
    with open(path, encoding="utf-8") as source:
        data = json.load(source)
except (OSError, json.JSONDecodeError) as error:
    raise SystemExit(f"invalid runtime metrics: {error}")
if not isinstance(data, dict):
    raise SystemExit("runtime metrics must contain a JSON object")
expected = {
    "_source_revision": revision,
    "_artifact_sha256": artifact_hash,
    "_installed_native_sha256": installed_hash,
    "_device_abi": abi,
    "_profile": profile,
    "_package": package,
}
for field, value in expected.items():
    if data.get(field) != value:
        raise SystemExit(f"runtime metrics binding mismatch: {field}")
samples = data.get("_samples")
if isinstance(samples, bool) or not isinstance(samples, int) or samples <= 0:
    raise SystemExit("runtime metrics contain no workload samples")
state = data.get("audio_power_state", "")
if not isinstance(state, str) or any(character.isspace() for character in state):
    raise SystemExit("runtime audio power state is malformed")
print(samples, state or "unreported")
PY
)" || die "runtime metrics are not bound to this release run"
read -r RUNTIME_SAMPLES AUDIO_POWER_STATE <<< "$RUNTIME_INFO"

NEED_POWER=false
if [[ "$METRIC_TYPE" == "power" || "$METRIC_TYPE" == "all" ]]; then
    NEED_POWER=true
    [[ "$AUDIO_POWER_STATE" == "idle" ]] || die "runtime did not attest the idle audio power state"
fi

read_proc_ticks() {
    local process_id="$1"
    local task_id="${2:-}"
    local proc_path="/proc/$process_id/stat"
    local stat_line stat_tail
    if [[ -n "$task_id" ]]; then
        proc_path="/proc/$process_id/task/$task_id/stat"
    fi
    stat_line="$("${ADB[@]}" shell "cat '$proc_path'" 2>/dev/null | tr -d '\r')" || return 1
    [[ "$stat_line" == *") "* ]] || return 1
    stat_tail="${stat_line##*) }"
    awk '{if ($12 !~ /^[0-9]+$/ || $13 !~ /^[0-9]+$/) exit 1; print $12 + $13}' <<< "$stat_tail"
}

read_memory_kb() {
    local process_id="$1"
    local meminfo native java total
    meminfo="$("${ADB[@]}" shell dumpsys meminfo "$process_id" 2>/dev/null | tr -d '\r')" || return 1
    native="$(awk '$1 == "Native" && $2 == "Heap" {print $3; exit}' <<< "$meminfo")"
    java="$(awk '$1 == "Java" && $2 == "Heap" {print $3; exit}' <<< "$meminfo")"
    total="$(awk '$1 == "TOTAL" && $2 == "PSS:" {print $3; exit}' <<< "$meminfo")"
    [[ "$native" =~ ^[0-9]+$ && "$java" =~ ^[0-9]+$ && "$total" =~ ^[0-9]+$ ]] || return 1
    printf '%s %s %s\n' "$native" "$java" "$total"
}

read_battery_field() {
    local field="$1"
    "${ADB[@]}" shell dumpsys battery 2>/dev/null | tr -d '\r' |
        awk -F: -v wanted="$field" '
            {key=$1; gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)}
            tolower(key) == tolower(wanted) {value=$2; gsub(/[[:space:]]/, "", value); print value; exit}'
}

read_thermal_status() {
    local thermal
    thermal="$("${ADB[@]}" shell dumpsys thermalservice 2>/dev/null | tr -d '\r')" || return 1
    awk '
        /Thermal Status:|mStatus:/ {
            for (i = NF; i > 0; --i) if ($i ~ /^[0-9]+$/) {print $i; exit}
        }' <<< "$thermal"
}

find_audio_tid() {
    local task_id comm found=""
    while IFS= read -r task_id; do
        task_id="${task_id//$'\r'/}"
        [[ "$task_id" =~ ^[0-9]+$ ]] || continue
        comm="$("${ADB[@]}" shell "cat /proc/$PID/task/$task_id/comm" 2>/dev/null | tr -d '\r\n')" || continue
        if [[ "$comm" == Migo-Audio* ]]; then
            [[ -z "$found" ]] || die "multiple Migo audio threads found"
            found="$task_id"
        fi
    done < <("${ADB[@]}" shell "ls -1 /proc/$PID/task" 2>/dev/null)
    [[ -n "$found" ]] || return 1
    printf '%s\n' "$found"
}

avg_values() {
    [[ $# -gt 0 ]] || return 1
    printf '%s\n' "$@" | awk '{sum += $1} END {printf "%.1f", sum / NR}'
}

max_values() {
    [[ $# -gt 0 ]] || return 1
    printf '%s\n' "$@" | awk 'NR == 1 || $1 > value {value=$1} END {printf "%.1f", value}'
}

SAMPLES_TAKEN=0
ELAPSED=0
CPU_AVG=0.0
CPU_PEAK=0.0
MEM_NATIVE_AVG=0.0
MEM_JAVA_AVG=0.0
MEM_TOTAL_AVG=0.0
BATTERY_DRAIN=0.0
AUDIO_IDLE_CPU=0.0
THERMAL_EVENTS=0

if [[ "$NEED_POWER" == true ]]; then
    CLK_TCK="$("${ADB[@]}" shell getconf CLK_TCK | tr -d '\r[:space:]')"
    [[ "$CLK_TCK" =~ ^[1-9][0-9]*$ ]] || die "device clock tick rate is unavailable"
    AUDIO_TID="$(find_audio_tid)" || die "Migo audio thread was not found"

    BATTERY_STATUS_START="$(read_battery_field status)"
    BATTERY_LEVEL="$(read_battery_field level)"
    BATTERY_SCALE="$(read_battery_field scale)"
    CHARGE_START="$(read_battery_field "charge counter")"
    [[ "$BATTERY_STATUS_START" == "3" ]] || die "device must be discharging during measurement"
    [[ "$BATTERY_LEVEL" =~ ^[1-9][0-9]*$ && "$BATTERY_SCALE" =~ ^[1-9][0-9]*$ ]] || \
        die "battery level/scale is unavailable"
    [[ "$CHARGE_START" =~ ^[1-9][0-9]*$ ]] || die "battery charge counter is unavailable"
    FULL_CHARGE_UAH="$("$SCRIPT_DIR/metric_math.py" full-charge \
        --charge-uah "$CHARGE_START" --level "$BATTERY_LEVEL" --scale "$BATTERY_SCALE")"

    PREV_PROCESS_TICKS="$(read_proc_ticks "$PID")" || die "could not read process CPU ticks"
    PREV_AUDIO_TICKS="$(read_proc_ticks "$PID" "$AUDIO_TID")" || die "could not read audio CPU ticks"
    PREV_TIME="$(date +%s)"
    PREV_THERMAL="$(read_thermal_status)"
    [[ "$PREV_THERMAL" =~ ^[0-9]+$ ]] || die "thermal status is unavailable"
    if (( PREV_THERMAL > 0 )); then
        THERMAL_EVENTS=1
    fi

    declare -a CPU_SAMPLES=()
    declare -a AUDIO_CPU_SAMPLES=()
    declare -a MEM_NATIVE_SAMPLES=()
    declare -a MEM_JAVA_SAMPLES=()
    declare -a MEM_TOTAL_SAMPLES=()

    START_TIME="$PREV_TIME"
    while (( $(date +%s) - START_TIME < DURATION )); do
        sleep "$INTERVAL"
        CURRENT_PID="$(get_pid)"
        [[ "$CURRENT_PID" == "$INITIAL_PID" ]] || die "process exited during measurement"

        CURRENT_TIME="$(date +%s)"
        DT=$((CURRENT_TIME - PREV_TIME))
        (( DT > 0 )) || die "measurement clock did not advance"
        CURRENT_PROCESS_TICKS="$(read_proc_ticks "$PID")" || die "process exited during measurement"
        CURRENT_AUDIO_TICKS="$(read_proc_ticks "$PID" "$AUDIO_TID")" || die "audio thread exited during measurement"
        (( CURRENT_PROCESS_TICKS >= PREV_PROCESS_TICKS )) || die "process CPU ticks moved backwards"
        (( CURRENT_AUDIO_TICKS >= PREV_AUDIO_TICKS )) || die "audio CPU ticks moved backwards"

        CPU_SAMPLES+=("$("$SCRIPT_DIR/metric_math.py" cpu \
            --delta-ticks "$((CURRENT_PROCESS_TICKS - PREV_PROCESS_TICKS))" \
            --clock-ticks "$CLK_TCK" --elapsed-seconds "$DT")")
        AUDIO_CPU_SAMPLES+=("$("$SCRIPT_DIR/metric_math.py" cpu \
            --delta-ticks "$((CURRENT_AUDIO_TICKS - PREV_AUDIO_TICKS))" \
            --clock-ticks "$CLK_TCK" --elapsed-seconds "$DT")")

        read -r NATIVE_KB JAVA_KB TOTAL_KB <<< "$(read_memory_kb "$PID")" || \
            die "process memory counters are unavailable"
        MEM_NATIVE_SAMPLES+=("$(awk -v value="$NATIVE_KB" 'BEGIN {printf "%.1f", value / 1024}')")
        MEM_JAVA_SAMPLES+=("$(awk -v value="$JAVA_KB" 'BEGIN {printf "%.1f", value / 1024}')")
        MEM_TOTAL_SAMPLES+=("$(awk -v value="$TOTAL_KB" 'BEGIN {printf "%.1f", value / 1024}')")

        CURRENT_THERMAL="$(read_thermal_status)"
        [[ "$CURRENT_THERMAL" =~ ^[0-9]+$ ]] || die "thermal status disappeared during measurement"
        if (( PREV_THERMAL == 0 && CURRENT_THERMAL > 0 )); then
            THERMAL_EVENTS=$((THERMAL_EVENTS + 1))
        fi
        PREV_THERMAL="$CURRENT_THERMAL"
        PREV_PROCESS_TICKS="$CURRENT_PROCESS_TICKS"
        PREV_AUDIO_TICKS="$CURRENT_AUDIO_TICKS"
        PREV_TIME="$CURRENT_TIME"
        SAMPLES_TAKEN=$((SAMPLES_TAKEN + 1))
    done

    (( SAMPLES_TAKEN > 0 )) || die "measurement produced no process samples"
    ELAPSED=$((PREV_TIME - START_TIME))
    (( ELAPSED > 0 )) || die "measurement duration is zero"
    BATTERY_STATUS_END="$(read_battery_field status)"
    CHARGE_END="$(read_battery_field "charge counter")"
    [[ "$BATTERY_STATUS_END" == "3" ]] || die "device stopped discharging during measurement"
    [[ "$CHARGE_END" =~ ^[1-9][0-9]*$ ]] || die "ending battery charge counter is unavailable"

    CPU_AVG="$(avg_values "${CPU_SAMPLES[@]}")"
    CPU_PEAK="$(max_values "${CPU_SAMPLES[@]}")"
    MEM_NATIVE_AVG="$(avg_values "${MEM_NATIVE_SAMPLES[@]}")"
    MEM_JAVA_AVG="$(avg_values "${MEM_JAVA_SAMPLES[@]}")"
    MEM_TOTAL_AVG="$(avg_values "${MEM_TOTAL_SAMPLES[@]}")"
    AUDIO_IDLE_CPU="$(avg_values "${AUDIO_CPU_SAMPLES[@]}")"
    BATTERY_DRAIN="$("$SCRIPT_DIR/metric_math.py" battery \
        --charge-start-uah "$CHARGE_START" --charge-end-uah "$CHARGE_END" \
        --full-charge-uah "$FULL_CHARGE_UAH" --elapsed-seconds "$ELAPSED")"
fi

AAR_SIZE_MB="$(python3 - "$ARTIFACT" <<'PY'
import os
import sys
print(f"{os.path.getsize(sys.argv[1]) / 1048576:.6f}")
PY
)"
mkdir -p "$(dirname "$OUTPUT")"

python3 - "$RUNTIME_METRICS" "$OUTPUT" "$METRIC_TYPE" "$RUNTIME_SAMPLES" \
    "$SAMPLES_TAKEN" "$SOURCE_REVISION" "$ARTIFACT" "$ARTIFACT_SHA256" \
    "$INSTALLED_NATIVE_SHA256" "$DEVICE_ABI" "$PROFILE" "$PACKAGE" \
    "$DEVICE_SERIAL" "$DEVICE_MODEL" "$ANDROID_API" "$SESSION_ID" "$ELAPSED" \
    "$AAR_SIZE_MB" "$SO_ARM64_SIZE_MB" "$CPU_AVG" "$CPU_PEAK" \
    "$MEM_NATIVE_AVG" "$MEM_JAVA_AVG" "$MEM_TOTAL_AVG" "$BATTERY_DRAIN" \
    "$AUDIO_IDLE_CPU" "$THERMAL_EVENTS" <<'PY'
import hashlib
import json
import math
import os
import sys
from datetime import datetime, timezone
from pathlib import Path

(
    runtime_path, output_path, metric_type, runtime_samples, process_samples,
    revision, artifact, artifact_hash, installed_hash, abi, profile, package,
    serial, model, android_api, session_id, elapsed, aar_size, arm64_size,
    cpu_avg, cpu_peak, native_mb, java_mb, total_mb, battery_drain,
    audio_idle_cpu, thermal_events,
) = sys.argv[1:]

with open(runtime_path, encoding="utf-8") as source:
    runtime = json.load(source)

document = {
    "_comment": "Artifact-bound Android release measurement",
    "_timestamp": datetime.now(timezone.utc).isoformat(),
    "_source_revision": revision,
    "_artifact": Path(artifact).name,
    "_artifact_sha256": artifact_hash,
    "_installed_native_sha256": installed_hash,
    "_device_abi": abi,
    "_profile": profile,
    "_package": package,
    "_device_serial_sha256": hashlib.sha256(serial.encode()).hexdigest(),
    "_device_model": model,
    "_android_api": int(android_api),
    "_session_id": int(session_id),
    "_duration_seconds": int(elapsed),
}

def finite_number(name, value):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise SystemExit(f"runtime metric is not numeric: {name}")
    if not math.isfinite(float(value)):
        raise SystemExit(f"runtime metric is not finite: {name}")
    return value

if metric_type in {"perf", "all"}:
    for name in (
        "fps", "frame_time_ms", "startup_time_ms", "first_frame_ms",
        "dropped_frames_pct", "command_drops",
    ):
        document[name] = finite_number(name, runtime.get(name))
    document["aar_size_mb"] = float(aar_size)
    document["so_arm64_size_mb"] = float(arm64_size)

if metric_type in {"power", "all"}:
    document.update({
        "cpu_avg_pct": float(cpu_avg),
        "cpu_peak_pct": float(cpu_peak),
        "memory_native_mb": float(native_mb),
        "memory_java_mb": float(java_mb),
        "memory_total_mb": float(total_mb),
        "battery_drain_pct_per_hour": float(battery_drain),
        "audio_idle_cpu_pct": float(audio_idle_cpu),
        "thermal_throttle_events": int(thermal_events),
    })

document["_samples"] = (
    int(runtime_samples) if metric_type == "perf"
    else int(process_samples) if metric_type == "power"
    else min(int(runtime_samples), int(process_samples))
)
if document["_samples"] <= 0:
    raise SystemExit("combined metrics contain no samples")

output = Path(output_path)
temporary = output.with_name(output.name + ".tmp")
temporary.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
os.replace(temporary, output)
PY

echo "Artifact-bound metrics written to: $OUTPUT"
