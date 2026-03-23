#!/usr/bin/env bash
# Collect runtime metrics from a connected Android device via adb.
#
# Usage:
#   bash scripts/ci/collect_metrics.sh [--type perf|power|all] [--output path.json]
#       [--package com.example.app] [--duration 30] [--interval 2]
#
# Prerequisites:
#   - adb in PATH with a single connected device
#   - Target app running with Migo runtime (debug build for DebugStats)
#
# Output: JSON file with flat key-value metrics suitable for compare_baseline.py
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# ── Defaults ──
METRIC_TYPE="all"
OUTPUT="ci/metrics/current_metrics.json"
PACKAGE=""
DURATION=30      # seconds to sample
INTERVAL=2       # seconds between samples
SESSION_ID=0

usage() {
    echo "Usage: $0 [--type perf|power|all] [--output path.json]"
    echo "          [--package com.example.app] [--duration 30] [--interval 2]"
    echo "          [--session-id 0]"
    exit 2
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --type)     METRIC_TYPE="$2"; shift 2 ;;
        --output)   OUTPUT="$2"; shift 2 ;;
        --package)  PACKAGE="$2"; shift 2 ;;
        --duration) DURATION="$2"; shift 2 ;;
        --interval) INTERVAL="$2"; shift 2 ;;
        --session-id) SESSION_ID="$2"; shift 2 ;;
        -h|--help)  usage ;;
        *)          echo "Unknown option: $1"; usage ;;
    esac
done

# ── Validate adb ──
if ! command -v adb &>/dev/null; then
    echo "ERROR: adb not found in PATH"
    exit 2
fi

DEVICE_COUNT=$(adb devices | grep -c 'device$' || true)
if [[ "$DEVICE_COUNT" -eq 0 ]]; then
    echo "ERROR: no connected Android device found"
    exit 2
fi

# ── Auto-detect package if not specified ──
if [[ -z "$PACKAGE" ]]; then
    # Try to find any process with libmigo.so loaded
    PACKAGE=$(adb shell "ps -A -o NAME" 2>/dev/null | grep -v '^NAME' | head -1 || true)
    if [[ -z "$PACKAGE" ]]; then
        echo "WARNING: could not auto-detect package, some metrics may be unavailable"
    fi
fi

echo "=== Migo Metrics Collection ==="
echo "  Type:     $METRIC_TYPE"
echo "  Package:  ${PACKAGE:-<auto>}"
echo "  Duration: ${DURATION}s"
echo "  Interval: ${INTERVAL}s"
echo "  Output:   $OUTPUT"
echo ""

# ── Helper: get PID ──
get_pid() {
    if [[ -n "$PACKAGE" ]]; then
        adb shell "pidof $PACKAGE" 2>/dev/null | tr -d '[:space:]' || true
    fi
}

# ── Helper: read /proc/self/stat for CPU ticks ──
# Returns: utime stime
read_cpu_ticks() {
    local pid="$1"
    if [[ -z "$pid" ]]; then
        echo "0 0"
        return
    fi
    # Fields 14 and 15 in /proc/[pid]/stat are utime and stime
    adb shell "cat /proc/$pid/stat" 2>/dev/null \
        | awk '{print $14, $15}' || echo "0 0"
}

# ── Helper: read memory info ──
read_memory() {
    local pid="$1"
    if [[ -z "$pid" ]]; then
        echo "0 0 0"
        return
    fi
    # dumpsys meminfo gives us native heap, java heap, total PSS
    local meminfo
    meminfo=$(adb shell "dumpsys meminfo $pid" 2>/dev/null || true)

    local native_heap java_heap total_pss
    native_heap=$(echo "$meminfo" | grep 'Native Heap' | head -1 | awk '{print $3}' || echo "0")
    java_heap=$(echo "$meminfo" | grep 'Java Heap' | head -1 | awk '{print $3}' || echo "0")
    total_pss=$(echo "$meminfo" | grep '^TOTAL PSS' | head -1 | awk '{print $3}' || echo "0")

    # Values from dumpsys are in KB
    echo "${native_heap:-0} ${java_heap:-0} ${total_pss:-0}"
}

# ── Helper: read battery level ──
read_battery() {
    adb shell "dumpsys battery" 2>/dev/null \
        | grep 'level:' | awk '{print $2}' | tr -d '[:space:]' || echo "0"
}

# ── Helper: get total CPU cores tick rate ──
get_clock_ticks() {
    adb shell "getconf CLK_TCK" 2>/dev/null | tr -d '[:space:]' || echo "100"
}

# ── Helper: get number of CPU cores ──
get_cpu_count() {
    adb shell "nproc" 2>/dev/null | tr -d '[:space:]' || echo "4"
}

# ── Collect samples ──
PID=$(get_pid)
CLK_TCK=$(get_clock_ticks)
NUM_CPUS=$(get_cpu_count)

echo "  PID:      ${PID:-<unknown>}"
echo "  CLK_TCK:  $CLK_TCK"
echo "  CPUs:     $NUM_CPUS"
echo ""

declare -a CPU_SAMPLES=()
declare -a MEM_NATIVE_SAMPLES=()
declare -a MEM_JAVA_SAMPLES=()
declare -a MEM_TOTAL_SAMPLES=()

# Initial readings
BATTERY_START=$(read_battery)
read -r PREV_UTIME PREV_STIME <<< "$(read_cpu_ticks "$PID")"
PREV_TIME=$(date +%s)

SAMPLES_TAKEN=0
ELAPSED=0

echo "Sampling..."
while [[ "$ELAPSED" -lt "$DURATION" ]]; do
    sleep "$INTERVAL"
    ELAPSED=$((ELAPSED + INTERVAL))

    PID=$(get_pid)
    if [[ -z "$PID" ]]; then
        echo "  WARNING: process not found at ${ELAPSED}s, skipping sample"
        continue
    fi

    # CPU
    read -r CUR_UTIME CUR_STIME <<< "$(read_cpu_ticks "$PID")"
    CUR_TIME=$(date +%s)
    DT=$((CUR_TIME - PREV_TIME))
    if [[ "$DT" -gt 0 && "$CLK_TCK" -gt 0 ]]; then
        DTICKS=$(( (CUR_UTIME - PREV_UTIME) + (CUR_STIME - PREV_STIME) ))
        # CPU% = (delta_ticks / CLK_TCK) / (delta_seconds * num_cpus) * 100
        # Use awk for floating point
        CPU_PCT=$(awk "BEGIN {printf \"%.1f\", ($DTICKS / $CLK_TCK) / ($DT * $NUM_CPUS) * 100}")
        CPU_SAMPLES+=("$CPU_PCT")
    fi
    PREV_UTIME=$CUR_UTIME
    PREV_STIME=$CUR_STIME
    PREV_TIME=$CUR_TIME

    # Memory (KB from dumpsys -> MB)
    read -r NATIVE_KB JAVA_KB TOTAL_KB <<< "$(read_memory "$PID")"
    MEM_NATIVE_SAMPLES+=($(awk "BEGIN {printf \"%.1f\", $NATIVE_KB / 1024}"))
    MEM_JAVA_SAMPLES+=($(awk "BEGIN {printf \"%.1f\", $JAVA_KB / 1024}"))
    MEM_TOTAL_SAMPLES+=($(awk "BEGIN {printf \"%.1f\", $TOTAL_KB / 1024}"))

    SAMPLES_TAKEN=$((SAMPLES_TAKEN + 1))
    echo "  Sample $SAMPLES_TAKEN at ${ELAPSED}s: CPU=${CPU_PCT:-?}% Native=${NATIVE_KB}KB Java=${JAVA_KB}KB"
done

BATTERY_END=$(read_battery)

echo ""
echo "Collected $SAMPLES_TAKEN samples over ${ELAPSED}s"
echo ""

# ── Compute aggregates ──
# Helper: average of array
avg() {
    local arr=("$@")
    if [[ ${#arr[@]} -eq 0 ]]; then
        echo "0"
        return
    fi
    local sum
    sum=$(printf '%s\n' "${arr[@]}" | awk '{s+=$1} END {printf "%.1f", s}')
    awk "BEGIN {printf \"%.1f\", $sum / ${#arr[@]}}"
}

# Helper: max of array
arr_max() {
    local arr=("$@")
    if [[ ${#arr[@]} -eq 0 ]]; then
        echo "0"
        return
    fi
    printf '%s\n' "${arr[@]}" | awk 'BEGIN{m=-999999} {if($1>m)m=$1} END{printf "%.1f", m}'
}

CPU_AVG=$(avg "${CPU_SAMPLES[@]}")
CPU_PEAK=$(arr_max "${CPU_SAMPLES[@]}")
MEM_NATIVE_AVG=$(avg "${MEM_NATIVE_SAMPLES[@]}")
MEM_JAVA_AVG=$(avg "${MEM_JAVA_SAMPLES[@]}")
MEM_TOTAL_AVG=$(avg "${MEM_TOTAL_SAMPLES[@]}")

# Battery drain per hour estimate
BATTERY_DRAIN=0
if [[ "$ELAPSED" -gt 0 ]]; then
    BATTERY_USED=$((BATTERY_START - BATTERY_END))
    BATTERY_DRAIN=$(awk "BEGIN {printf \"%.1f\", $BATTERY_USED / $ELAPSED * 3600}")
fi

echo "=== Aggregated Metrics ==="
echo "  CPU avg:       ${CPU_AVG}%"
echo "  CPU peak:      ${CPU_PEAK}%"
echo "  Memory native: ${MEM_NATIVE_AVG} MB"
echo "  Memory java:   ${MEM_JAVA_AVG} MB"
echo "  Memory total:  ${MEM_TOTAL_AVG} MB"
echo "  Battery drain: ${BATTERY_DRAIN}%/hr"
echo ""

# ── Write output JSON ──
OUTPUT_DIR=$(dirname "$OUTPUT")
mkdir -p "$OUTPUT_DIR"

cat > "$OUTPUT" <<JSONEOF
{
  "_comment": "Auto-collected by collect_metrics.sh",
  "_timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "_device": "$(adb shell getprop ro.product.model 2>/dev/null | tr -d '[:space:]')",
  "_android_version": "$(adb shell getprop ro.build.version.release 2>/dev/null | tr -d '[:space:]')",
  "_duration_seconds": $ELAPSED,
  "_samples": $SAMPLES_TAKEN,
  "cpu_avg_pct": $CPU_AVG,
  "cpu_peak_pct": $CPU_PEAK,
  "memory_native_mb": $MEM_NATIVE_AVG,
  "memory_java_mb": $MEM_JAVA_AVG,
  "memory_total_mb": $MEM_TOTAL_AVG,
  "battery_drain_pct_per_hour": $BATTERY_DRAIN
}
JSONEOF

echo "Metrics written to: $OUTPUT"
