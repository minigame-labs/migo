#!/usr/bin/env bash
# Compare current power/resource metrics against baseline thresholds.
#
# Usage:
#   bash scripts/ci/run_power.sh \
#       --current  ci/metrics/current_power_metrics.json \
#       --baseline ci/baselines/android_power_default.json \
#       --summary-out reports/power-compare-summary.json
#
# This is a thin wrapper around compare_baseline.py.
# Exit codes: 0 = pass, 1 = regression, 2 = usage error
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── Defaults ──
CURRENT=""
BASELINE="ci/baselines/android_power_default.json"
SUMMARY_OUT=""
EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --current)     CURRENT="$2"; shift 2 ;;
        --baseline)    BASELINE="$2"; shift 2 ;;
        --summary-out) SUMMARY_OUT="$2"; shift 2 ;;
        --fail-on-warn) EXTRA_ARGS+=("--fail-on-warn"); shift ;;
        -h|--help)
            echo "Usage: $0 --current <path> [--baseline <path>] [--summary-out <path>] [--fail-on-warn]"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 2 ;;
    esac
done

if [[ -z "$CURRENT" ]]; then
    echo "ERROR: --current is required"
    exit 2
fi

echo "=== Power Baseline Comparison ==="

CMD=(python3 "$SCRIPT_DIR/compare_baseline.py"
    --current "$CURRENT"
    --baseline "$BASELINE"
)

if [[ -n "$SUMMARY_OUT" ]]; then
    CMD+=(--summary-out "$SUMMARY_OUT")
fi

CMD+=("${EXTRA_ARGS[@]}")

exec "${CMD[@]}"
