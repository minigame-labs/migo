#!/usr/bin/env python3

import argparse
import json
import sys
from pathlib import Path


DEFAULT_THRESHOLDS = {
    "avg_fps_min_delta": -2.0,
    "p95_ms_max_delta": 3.0,
    "first_frame_ms_max_delta": 30.0,
    "full_surface_frames_max_delta": 50.0,
    "upload_frame_rejections_max_delta": 20.0,
}


def compare_result(baseline, current, thresholds=None):
    thresholds = {**DEFAULT_THRESHOLDS, **(thresholds or {})}
    reasons = []

    if "avg_fps" in baseline and "avg_fps" in current:
        if current["avg_fps"] < baseline["avg_fps"] + thresholds["avg_fps_min_delta"]:
            reasons.append("avg_fps")

    if "p95_ms" in baseline and "p95_ms" in current:
        if current["p95_ms"] > baseline["p95_ms"] + thresholds["p95_ms_max_delta"]:
            reasons.append("p95_ms")

    if "first_frame_ms" in baseline and "first_frame_ms" in current:
        if current["first_frame_ms"] > baseline["first_frame_ms"] + thresholds["first_frame_ms_max_delta"]:
            reasons.append("first_frame_ms")

    # Render optimization regression checks — more full-surface or more
    # upload rejections than baseline means an optimization regressed.
    if "full_surface_frames" in baseline and "full_surface_frames" in current:
        if current["full_surface_frames"] > baseline["full_surface_frames"] + thresholds["full_surface_frames_max_delta"]:
            reasons.append("full_surface_frames")

    if "upload_frame_rejections" in baseline and "upload_frame_rejections" in current:
        if current["upload_frame_rejections"] > baseline["upload_frame_rejections"] + thresholds["upload_frame_rejections_max_delta"]:
            reasons.append("upload_frame_rejections")

    return {"pass": not reasons, "reasons": reasons}


def load_json(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def build_baseline_lookup(baseline_rows):
    lookup = {}
    duplicates = []
    for row in baseline_rows:
        if not isinstance(row, dict):
            continue
        device = row.get("device")
        workload = row.get("workload")
        stats = row.get("stats")
        if device is None or workload is None or not isinstance(stats, dict):
            continue
        key = (device, workload)
        if key in lookup:
            duplicates.append({"device": device, "workload": workload})
            continue
        lookup[key] = stats
    return lookup, duplicates


def is_valid_current_row(result):
    return (
        isinstance(result, dict)
        and isinstance(result.get("device"), str)
        and bool(result.get("device"))
        and isinstance(result.get("workload"), str)
        and bool(result.get("workload"))
        and isinstance(result.get("stats"), dict)
    )


def compare_results(results, baseline_doc):
    thresholds = {**DEFAULT_THRESHOLDS, **baseline_doc.get("thresholds", {})}
    baseline_lookup, duplicates = build_baseline_lookup(baseline_doc.get("results", []))
    summary = []
    has_failure = False
    matched_rows = 0
    seen_keys = set()
    reasons = []

    if duplicates:
        has_failure = True
        reasons.append("duplicate baseline key")

    if not results:
        has_failure = True
        reasons.append("empty current results")

    for result in results:
        if not isinstance(result, dict):
            summary.append({"pass": False, "reasons": ["invalid_row"]})
            has_failure = True
            reasons.append("malformed current row")
            continue

        device = result.get("device")
        workload = result.get("workload")
        current = result.get("stats")
        entry = {"device": device, "workload": workload}

        if not is_valid_current_row(result):
            entry.update({"pass": False, "reasons": ["invalid_row"]})
            summary.append(entry)
            has_failure = True
            reasons.append("malformed current row")
            continue

        baseline = baseline_lookup.get((device, workload))
        if baseline is None:
            entry.update({"pass": True, "reasons": [], "skipped": True})
            summary.append(entry)
            continue

        comparison = compare_result(baseline, current, thresholds)
        entry.update(comparison)
        matched_rows += 1
        seen_keys.add((device, workload))
        if not comparison["pass"]:
            has_failure = True
        summary.append(entry)

    if summary and matched_rows == 0:
        has_failure = True
        reasons.append("no baseline matches found")

    missing_keys = [key for key in baseline_lookup if key not in seen_keys]
    if missing_keys:
        has_failure = True
        reasons.append("missing baseline rows in current results")

    # Detect thresholds that reference metrics absent from ALL baseline rows.
    # This catches the "threshold declared but baseline never has the field"
    # gap that silently skips comparison.
    uncovered = _find_uncovered_thresholds(thresholds, baseline_lookup)
    if uncovered:
        has_failure = True
        reasons.append("uncovered thresholds")

    report = {"pass": not has_failure, "reasons": reasons, "results": summary}
    if uncovered:
        report["uncovered_thresholds"] = sorted(uncovered)
    return report


def _find_uncovered_thresholds(thresholds, baseline_lookup):
    """Return metric names whose thresholds exist but no baseline row has the field."""
    # Extract metric names from threshold keys like "full_surface_frames_max_delta".
    threshold_metrics = set()
    for key in thresholds:
        for suffix in ("_max_delta", "_min_delta"):
            if key.endswith(suffix):
                threshold_metrics.add(key[: -len(suffix)])
                break
    # Check which metrics appear in at least one baseline row.
    covered = set()
    for stats in baseline_lookup.values():
        for metric in threshold_metrics:
            if metric in stats:
                covered.add(metric)
    return threshold_metrics - covered


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("--current", required=True)
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--summary-out")
    args = parser.parse_args(argv)

    current = load_json(args.current)
    if not isinstance(current, list):
        raise ValueError("current render results must be a JSON array")

    baseline_doc = load_json(args.baseline)
    if not isinstance(baseline_doc, dict):
        raise ValueError("baseline render results must be a JSON object")

    report = compare_results(current, baseline_doc)

    if args.summary_out:
        summary_path = Path(args.summary_out)
        summary_path.parent.mkdir(parents=True, exist_ok=True)
        summary_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    print(json.dumps(report, indent=2))
    return 0 if report["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
