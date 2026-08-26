import argparse
import json
from pathlib import Path


def _require_mapping(value, context):
    if not isinstance(value, dict):
        raise ValueError(f"{context} must be an object")
    return value


def _require_field(mapping, field, context):
    if field not in mapping:
        raise ValueError(f"missing required field: {context}{field}")
    return mapping[field]


def _require_float(mapping, field, context):
    value = _require_field(mapping, field, context)
    try:
        return float(value)
    except (TypeError, ValueError) as exc:
        raise ValueError(f"{context}{field} must be a number") from exc


def normalize_result(raw):
    raw = _require_mapping(raw, "result")
    stats = _require_mapping(_require_field(raw, "stats", ""), "stats")
    normalized_stats = {
        "avg_fps": _require_float(stats, "avg_fps", "stats."),
        "p95_ms": _require_float(stats, "p95_ms", "stats."),
    }
    # Optional metrics — preserved when present.
    for key in (
        "first_frame_ms",
        "partial_damage_frames",
        "full_surface_frames",
        "damage_area_k_pixels",
        "upload_frame_rejections",
        "dropped_upload_recoveries",
    ):
        if key in stats:
            normalized_stats[key] = _require_float(stats, key, "stats.")
    result = {
        "device": _require_field(raw, "device", ""),
        "workload": _require_field(raw, "workload", ""),
        "stats": normalized_stats,
    }
    for field in ("_source_revision", "_artifact_sha256", "_profile"):
        if field in raw:
            result[field] = raw[field]
    return result


def main(src, dst):
    src_path = Path(src)
    dst_path = Path(dst)
    data = json.loads(src_path.read_text(encoding="utf-8"))
    if not isinstance(data, list):
        raise ValueError("top-level JSON must be an array of results")
    normalized = [normalize_result(item) for item in data]
    dst_path.parent.mkdir(parents=True, exist_ok=True)
    dst_path.write_text(
        json.dumps(normalized, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("src")
    parser.add_argument("dst")
    args = parser.parse_args()
    main(args.src, args.dst)
