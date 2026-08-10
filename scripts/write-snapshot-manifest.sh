#!/usr/bin/env bash
# Writes <bin>.manifest.json next to a generated V8 snapshot, capturing the
# schema/profile/feature/JS/Rust/deno fingerprint and release identity that
# check-snapshot-freshness.sh compares against.
#
# Shared by gen-snapshot.sh (arm64, on a real device) and build-snapshot.yml
# (x86_64, in CI) so BOTH arches emit byte-identical manifest formats and
# identical fingerprints — that is the whole point of the freshness gate.
#
# Usage: write-snapshot-manifest.sh <profile> <arch> <snapshot-bin> [host|worker]
set -euo pipefail

PRODUCT_PROFILE="${1:?usage: write-snapshot-manifest.sh <profile> <arch> <bin-path>}"
ARCH="${2:?usage: write-snapshot-manifest.sh <profile> <arch> <bin-path>}"
BIN="${3:?usage: write-snapshot-manifest.sh <profile> <arch> <bin-path>}"
SNAPSHOT_KIND="${4:-host}"
[[ -s "$BIN" ]] || { echo "ERROR: snapshot missing or empty: $BIN" >&2; exit 1; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Derive the repo root from this script's own location (scripts/..), NOT
# `git rev-parse` — the latter exits 128 without an accessible `.git`, which
# would abort gen-snapshot's manifest step under `set -e`. Matches how
# check-snapshot-freshness.sh locates ROOT so both agree with no git dependency.
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE="$ROOT/engine"

# shellcheck source=scripts/lib/snapshot-fingerprint.sh
source "$ROOT/scripts/lib/snapshot-fingerprint.sh"
source "$ROOT/scripts/lib/reproducible-timestamp.sh"
snapshot_validate_kind_profile "$SNAPSHOT_KIND" "$PRODUCT_PROFILE" || exit 1
snapshot_require_materialized_snapshot "$BIN" || exit 1
JS_HASH="$(snapshot_js_hash "$ROOT")"
RUST_HASH="$(snapshot_runtime_hash "$ROOT")"
FEATURE_HASH="$(snapshot_feature_hash "$PRODUCT_PROFILE")"
FEATURES_JSON="$(snapshot_profile_features_json "$PRODUCT_PROFILE")"
DENO_CORE_VER="$(snapshot_deno_core_version "$ENGINE")"
V8_ARCHIVE="${MIGO_V8_ARCHIVE:-$ENGINE/third_party/rusty_v8/$ARCH/librusty_v8.a}"
snapshot_require_materialized_v8_archive "$V8_ARCHIVE" || exit 1
V8_HASH="$(snapshot_v8_archive_hash "$V8_ARCHIVE")"

case "$ARCH" in
  aarch64) TARGET_TRIPLE="aarch64-linux-android" ;;
  x86_64) TARGET_TRIPLE="x86_64-linux-android" ;;
  *) echo "ERROR: unsupported Android snapshot arch: $ARCH" >&2; exit 1 ;;
esac
NORMALIZED_PARAMETERS_JSON="[\"--arch=$ARCH\",\"--cpu-policy=target-baseline\",\"--product-profile=$PRODUCT_PROFILE\",\"--runtime-kind=$SNAPSHOT_KIND\",\"--warmup=none\"]"
# The Rust source fingerprint conservatively covers the external-reference/op
# tables. The bootstrap identity binds every input consumed by snapshot-gen.
EXTERNAL_REFERENCES_HASH="$RUST_HASH"
BOOTSTRAP_INPUTS_HASH="$({
  printf '%s\n' \
    "schema=$SNAPSHOT_SCHEMA_VERSION" \
    "snapshot_kind=$SNAPSHOT_KIND" \
    "profile=$PRODUCT_PROFILE" \
    "arch=$ARCH" \
    "target_triple=$TARGET_TRIPLE" \
    "parameters=$NORMALIZED_PARAMETERS_JSON" \
    "features=$FEATURE_HASH" \
    "rust=$RUST_HASH" \
    "js=$JS_HASH" \
    "v8=$V8_HASH" \
    "deno_core=$DENO_CORE_VER"
} | sha256sum | awk '{print $1}')"

MANIFEST="$BIN.manifest.json"
cat > "$MANIFEST" <<EOF
{
  "schema_version": $SNAPSHOT_SCHEMA_VERSION,
  "snapshot_kind": "$SNAPSHOT_KIND",
  "profile": "$PRODUCT_PROFILE",
  "arch": "$ARCH",
  "target_triple": "$TARGET_TRIPLE",
  "generation_cpu_policy": "target-baseline",
  "normalized_parameters": $NORMALIZED_PARAMETERS_JSON,
  "external_references_sha256": "$EXTERNAL_REFERENCES_HASH",
  "bootstrap_inputs_sha256": "$BOOTSTRAP_INPUTS_HASH",
  "features": $FEATURES_JSON,
  "features_sha256": "$FEATURE_HASH",
  "rust_sources_sha256": "$RUST_HASH",
  "v8_archive_sha256": "$V8_HASH",
  "snapshot_size": $(stat -c %s "$BIN"),
  "snapshot_sha256": "$(sha256sum "$BIN" | awk '{print $1}')",
  "js_sources_sha256": "$JS_HASH",
  "deno_core_version": "$DENO_CORE_VER",
  "git_commit": "$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)",
  "generated_at": "$(reproducible_timestamp)"
}
EOF
echo "manifest -> $MANIFEST"
