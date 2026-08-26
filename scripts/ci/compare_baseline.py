#!/usr/bin/env python3
"""
Compare collected runtime metrics against baseline thresholds.

Usage:
    python scripts/ci/compare_baseline.py \\
        --current  ci/metrics/current_metrics.json \\
        --baseline ci/baselines/android_perf_default.json \\
        --summary-out reports/perf-compare-summary.json

Exit codes:
    0 - all metrics pass (may include warnings)
    1 - one or more metrics exceed fail threshold (regression)
    2 - usage / file error

The current metrics JSON should have flat key-value pairs matching the
metric names defined in the baseline file, e.g.:
    { "fps": 58.3, "frame_time_ms": 16.8, "startup_time_ms": 2100 }

Metrics marked `required` in the baseline fail when missing or non-numeric.
Only explicitly optional metrics may be skipped.
"""

import argparse
import json
import math
import os
import sys
from datetime import datetime, timezone

REPORT_BINDING_FIELDS = (
    "_source_revision",
    "_artifact_sha256",
    "_installed_native_sha256",
    "_device_abi",
    "_profile",
    "_package",
)


def load_json(path):
    """Load and parse a JSON file."""
    try:
        with open(path, "r") as f:
            return json.load(f)
    except FileNotFoundError:
        print(f"ERROR: file not found: {path}", file=sys.stderr)
        sys.exit(2)
    except json.JSONDecodeError as e:
        print(f"ERROR: invalid JSON in {path}: {e}", file=sys.stderr)
        sys.exit(2)


def evaluate_metric(name, value, spec):
    """
    Evaluate a single metric against its baseline spec.

    Returns a dict: { status: pass|warn|fail, value, message, threshold }
    """
    direction = spec.get("direction", "lower_is_better")
    unit = spec.get("unit", "")
    result = {"name": name, "value": value, "unit": unit, "direction": direction}

    if direction == "lower_is_better":
        fail_threshold = spec.get("fail_above")
        warn_threshold = spec.get("warn_above")
        if fail_threshold is not None and value > fail_threshold:
            result["status"] = "fail"
            result["threshold"] = fail_threshold
            result["message"] = f"{name} = {value}{unit} exceeds fail threshold {fail_threshold}{unit}"
        elif warn_threshold is not None and value > warn_threshold:
            result["status"] = "warn"
            result["threshold"] = warn_threshold
            result["message"] = f"{name} = {value}{unit} exceeds warn threshold {warn_threshold}{unit}"
        else:
            result["status"] = "pass"
            result["message"] = f"{name} = {value}{unit} OK"
            result["threshold"] = fail_threshold
    elif direction == "higher_is_better":
        fail_threshold = spec.get("fail_below")
        warn_threshold = spec.get("warn_below")
        if fail_threshold is not None and value < fail_threshold:
            result["status"] = "fail"
            result["threshold"] = fail_threshold
            result["message"] = f"{name} = {value}{unit} below fail threshold {fail_threshold}{unit}"
        elif warn_threshold is not None and value < warn_threshold:
            result["status"] = "warn"
            result["threshold"] = warn_threshold
            result["message"] = f"{name} = {value}{unit} below warn threshold {warn_threshold}{unit}"
        else:
            result["status"] = "pass"
            result["message"] = f"{name} = {value}{unit} OK"
            result["threshold"] = fail_threshold
    else:
        result["status"] = "fail"
        result["threshold"] = None
        result["message"] = f"{name}: unknown comparison direction {direction!r}"

    return result


def compare(current, baseline):
    """
    Compare all metrics. Returns (results_list, has_failures, has_warnings).
    """
    metrics_spec = baseline.get("metrics", {})
    results = []
    has_failures = False
    has_warnings = False

    for metric_name, spec in sorted(metrics_spec.items()):
        required = spec.get("required") is True
        if metric_name not in current:
            status = "fail" if required else "skipped"
            results.append({
                "name": metric_name,
                "status": status,
                "message": (
                    f"{metric_name}: required metric is missing"
                    if required
                    else f"{metric_name}: optional metric is missing (skipped)"
                ),
            })
            has_failures = has_failures or required
            continue

        value = current[metric_name]
        numeric = (
            isinstance(value, (int, float))
            and not isinstance(value, bool)
            and math.isfinite(float(value))
        )
        if not numeric:
            status = "fail" if required else "skipped"
            results.append({
                "name": metric_name,
                "status": status,
                "message": (
                    f"{metric_name}: required metric is not a finite number"
                    if required
                    else f"{metric_name}: optional metric is not numeric (skipped)"
                ),
            })
            has_failures = has_failures or required
            continue

        result = evaluate_metric(metric_name, value, spec)
        results.append(result)

        if result["status"] == "fail":
            has_failures = True
        elif result["status"] == "warn":
            has_warnings = True

    return results, has_failures, has_warnings


def print_report(results, has_failures, has_warnings):
    """Print a human-readable report to stdout."""
    status_icon = {"pass": "  OK ", "warn": "WARN ", "fail": "FAIL ", "skipped": "SKIP "}

    print("=" * 70)
    print("  Baseline Comparison Report")
    print("=" * 70)
    print()

    for r in results:
        icon = status_icon.get(r["status"], "???? ")
        print(f"  [{icon}] {r['message']}")

    print()
    print("-" * 70)

    counts = {"pass": 0, "warn": 0, "fail": 0, "skipped": 0}
    for r in results:
        counts[r["status"]] = counts.get(r["status"], 0) + 1

    print(f"  Total: {len(results)}  |  Pass: {counts['pass']}  |  Warn: {counts['warn']}  |  Fail: {counts['fail']}  |  Skip: {counts['skipped']}")

    if has_failures:
        print("\n  RESULT: FAIL (one or more metrics exceed fail threshold)")
    elif has_warnings:
        print("\n  RESULT: PASS with warnings")
    else:
        print("\n  RESULT: PASS")
    print("=" * 70)


def write_summary(
    path,
    results,
    has_failures,
    has_warnings,
    baseline_path,
    current_path,
    current,
):
    """Write machine-readable summary JSON."""
    summary = {
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "baseline_file": baseline_path,
        "current_file": current_path,
        "overall": "fail" if has_failures else ("warn" if has_warnings else "pass"),
        "metrics": results,
    }
    summary.update(
        {field: current[field] for field in REPORT_BINDING_FIELDS if field in current}
    )

    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        json.dump(summary, f, indent=2)
    print(f"\n  Summary written to: {path}")


def main():
    parser = argparse.ArgumentParser(
        description="Compare runtime metrics against baseline thresholds."
    )
    parser.add_argument(
        "--current", required=True,
        help="Path to current metrics JSON (flat key-value pairs)",
    )
    parser.add_argument(
        "--baseline", required=True,
        help="Path to baseline thresholds JSON",
    )
    parser.add_argument(
        "--summary-out", default=None,
        help="Path to write summary JSON (optional)",
    )
    parser.add_argument(
        "--fail-on-warn", action="store_true",
        help="Treat warnings as failures (exit 1)",
    )

    args = parser.parse_args()

    current = load_json(args.current)
    baseline = load_json(args.baseline)

    results, has_failures, has_warnings = compare(current, baseline)

    print_report(results, has_failures, has_warnings)

    if args.summary_out:
        write_summary(
            args.summary_out,
            results,
            has_failures,
            has_warnings,
            args.baseline,
            args.current,
            current,
        )

    if has_failures:
        sys.exit(1)
    if has_warnings and args.fail_on_warn:
        sys.exit(1)
    sys.exit(0)


if __name__ == "__main__":
    main()
