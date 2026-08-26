#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

resolve_path() {
  local path="$1"
  if [[ "$path" = /* ]]; then
    printf '%s\n' "$path"
  else
    printf '%s/%s\n' "$REPO_ROOT" "$path"
  fi
}

MANIFEST="$(resolve_path "${1:-ci/render_workloads_default.json}")"
OUT_DIR="$(resolve_path "${2:-artifacts/render-matrix}")"
SOURCE_REVISION="${3:-}"
ARTIFACT_SHA256="${4:-}"
PROFILE="${5:-}"
RAW_RESULTS="$OUT_DIR/raw-results.json"
NORMALIZED_RESULTS="$OUT_DIR/normalized-results.json"
RENDER_BASELINE="$REPO_ROOT/ci/baselines/android_render_default.json"
COMPARISON_SUMMARY="$OUT_DIR/render-compare-summary.json"

if [[ ! -f "$MANIFEST" ]]; then
  printf 'manifest not found: %s\n' "$MANIFEST" >&2
  exit 1
fi

if [[ ! "$SOURCE_REVISION" =~ ^[0-9a-f]{40}$ ]]; then
  printf 'full source revision is required as argument 3\n' >&2
  exit 2
fi
if [[ ! "$ARTIFACT_SHA256" =~ ^[0-9a-f]{64}$ ]]; then
  printf 'artifact SHA-256 is required as argument 4\n' >&2
  exit 2
fi
if [[ "$PROFILE" != "full" ]]; then
  printf 'full profile is required as argument 5\n' >&2
  exit 2
fi

mkdir -p "$OUT_DIR"

if [[ ! -f "$RAW_RESULTS" ]]; then
  printf 'raw results not found: %s\n' "$RAW_RESULTS" >&2
  exit 1
fi

python3 "$REPO_ROOT/scripts/ci/collect_render_metrics.py" "$RAW_RESULTS" "$NORMALIZED_RESULTS"
python3 "$REPO_ROOT/scripts/ci/compare_render_results.py" \
  --current "$NORMALIZED_RESULTS" \
  --baseline "$RENDER_BASELINE" \
  --summary-out "$COMPARISON_SUMMARY" \
  --source-revision "$SOURCE_REVISION" \
  --artifact-sha256 "$ARTIFACT_SHA256" \
  --profile "$PROFILE"
