#!/usr/bin/env bash
# Verify profile-qualified Android V8 snapshots without a device.
set -euo pipefail

c_info() { echo -e "\033[0;36m[INFO] $*\033[0m"; }
c_ok()   { echo -e "\033[0;32m[OK]   $*\033[0m"; }
c_warn() { echo -e "\033[0;33m[WARN] $*\033[0m"; }
c_err()  { echo -e "\033[0;31m[STALE] $*\033[0m" >&2; }
die()    { c_err "$*"; exit 2; }

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENGINE="$ROOT/engine"
SNAP_DIR="$ENGINE/crates/js-runtime/snapshots"
# shellcheck source=scripts/lib/snapshot-fingerprint.sh
source "$ROOT/scripts/lib/snapshot-fingerprint.sh"

PRODUCT_PROFILE="full"
SNAPSHOT_KIND="host"
ARCHES=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --product-profile)
      [[ $# -ge 2 ]] || die "--product-profile requires full|slim"
      PRODUCT_PROFILE="$2"; shift 2
      ;;
    --product-profile=*) PRODUCT_PROFILE="${1#*=}"; shift ;;
    --snapshot-kind)
      [[ $# -ge 2 ]] || die "--snapshot-kind requires host|worker"
      SNAPSHOT_KIND="$2"; shift 2
      ;;
    --snapshot-kind=*) SNAPSHOT_KIND="${1#*=}"; shift ;;
    -h|--help)
      echo "usage: $0 [--product-profile full|slim] [--snapshot-kind host|worker] [aarch64|x86_64 ...]"
      exit 0
      ;;
    *) ARCHES+=("$1"); shift ;;
  esac
done
case "$PRODUCT_PROFILE" in
  full|slim) ;;
  *) die "invalid product profile '$PRODUCT_PROFILE' (expected full|slim)" ;;
esac
snapshot_validate_kind_profile "$SNAPSHOT_KIND" "$PRODUCT_PROFILE" || exit 2

if [[ "$SNAPSHOT_KIND" == "host" ]]; then
  SNAPSHOT_PREFIX="SNAPSHOT-$PRODUCT_PROFILE"
else
  SNAPSHOT_PREFIX="SNAPSHOT-worker-$PRODUCT_PROFILE"
fi

CUR_JS="$(snapshot_js_hash "$ROOT")"
CUR_RUST="$(snapshot_runtime_hash "$ROOT")"
CUR_FEATURES="$(snapshot_feature_hash "$PRODUCT_PROFILE")"
CUR_DENO="$(snapshot_deno_core_version "$ENGINE")"
c_info "kind=$SNAPSHOT_KIND profile=$PRODUCT_PROFILE schema=$SNAPSHOT_SCHEMA_VERSION deno_core=$CUR_DENO js=${CUR_JS:0:12} rust=${CUR_RUST:0:12}"

if [[ "${#ARCHES[@]}" -eq 0 ]]; then
  discovered_arches=()
  for file in "$SNAP_DIR"/"$SNAPSHOT_PREFIX"-*.bin; do
    [[ -e "$file" ]] || continue
    arch="${file##*/$SNAPSHOT_PREFIX-}"
    discovered_arches+=("${arch%.bin}")
  done
  mapfile -t ARCHES < <(snapshot_default_arches "$SNAPSHOT_KIND" "${discovered_arches[@]}")
fi

jget() {
  sed -n "s/.*\"$2\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" "$1" | head -1
}
jget_num() {
  sed -n "s/.*\"$2\"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p" "$1" | head -1
}

stale=0
if [[ "$SNAPSHOT_KIND" == "host" ]]; then
  for legacy in "$SNAP_DIR"/SNAPSHOT-aarch64.bin "$SNAP_DIR"/SNAPSHOT-x86_64.bin; do
    if [[ -e "$legacy" ]]; then
      c_err "$(basename "$legacy"): legacy arch-only snapshot is schema-incompatible and will not be embedded"
      stale=1
    fi
  done
fi

if [[ "${#ARCHES[@]}" -eq 0 ]]; then
  if [[ "$SNAPSHOT_KIND" == "worker" ]]; then
    c_warn "no worker/full snapshots present; default builds use source bootstrap and an explicit candidate fails closed"
  else
    c_warn "no host/$PRODUCT_PROFILE snapshots present; builds use safe source-JS fallback"
  fi
  [[ "$stale" -eq 0 ]] || exit 1
  exit 0
fi

for arch in "${ARCHES[@]}"; do
  snap="$SNAP_DIR/$SNAPSHOT_PREFIX-$arch.bin"
  man="$snap.manifest.json"
  [[ -s "$snap" ]] || { c_err "$PRODUCT_PROFILE-$arch: snapshot missing/empty"; stale=1; continue; }
  [[ -f "$man" ]] || { c_err "$PRODUCT_PROFILE-$arch: manifest missing"; stale=1; continue; }

  m_schema="$(jget_num "$man" schema_version)"
  m_kind="$(jget "$man" snapshot_kind)"
  m_profile="$(jget "$man" profile)"
  m_arch="$(jget "$man" arch)"
  m_features="$(jget "$man" features_sha256)"
  m_rust="$(jget "$man" rust_sources_sha256)"
  m_v8="$(jget "$man" v8_archive_sha256)"
  m_js="$(jget "$man" js_sources_sha256)"
  m_deno="$(jget "$man" deno_core_version)"
  m_snap="$(jget "$man" snapshot_sha256)"
  m_snap_size="$(jget_num "$man" snapshot_size)"
  if ! actual_snap="$(snapshot_artifact_hash "$snap")" ||
     ! actual_snap_size="$(snapshot_artifact_size "$snap")"; then
    c_err "$PRODUCT_PROFILE-$arch: snapshot identity unavailable ($snap)"
    stale=1
    continue
  fi
  v8_archive="$ENGINE/third_party/rusty_v8/$arch/librusty_v8.a"
  if ! actual_v8="$(snapshot_v8_archive_hash "$v8_archive")"; then
    c_err "$PRODUCT_PROFILE-$arch: V8 archive identity unavailable ($v8_archive)"
    stale=1
    continue
  fi

  if [[ "$m_schema" != "$SNAPSHOT_SCHEMA_VERSION" ||
        "$m_kind" != "$SNAPSHOT_KIND" ||
        "$m_profile" != "$PRODUCT_PROFILE" || "$m_arch" != "$arch" ||
        "$m_features" != "$CUR_FEATURES" || "$m_rust" != "$CUR_RUST" ||
        "$m_v8" != "$actual_v8" ||
        "$m_js" != "$CUR_JS" || "$m_deno" != "$CUR_DENO" ||
        "$m_snap" != "$actual_snap" || "$m_snap_size" != "$actual_snap_size" ]]; then
    c_err "$PRODUCT_PROFILE-$arch: incompatible or stale"
    [[ "$m_schema" != "$SNAPSHOT_SCHEMA_VERSION" ]] && echo "        schema: manifest=${m_schema:-missing} current=$SNAPSHOT_SCHEMA_VERSION"
    [[ "$m_kind" != "$SNAPSHOT_KIND" ]] && echo "        kind: manifest=${m_kind:-missing} current=$SNAPSHOT_KIND"
    [[ "$m_profile" != "$PRODUCT_PROFILE" ]] && echo "        profile: manifest=${m_profile:-missing} current=$PRODUCT_PROFILE"
    [[ "$m_arch" != "$arch" ]] && echo "        arch: manifest=${m_arch:-missing} current=$arch"
    [[ "$m_features" != "$CUR_FEATURES" ]] && echo "        product feature set changed"
    [[ "$m_rust" != "$CUR_RUST" ]] && echo "        js-runtime Rust/op sources changed"
    [[ "$m_v8" != "$actual_v8" ]] && echo "        Android V8 archive/build changed"
    [[ "$m_js" != "$CUR_JS" ]] && echo "        extension JS changed"
    [[ "$m_deno" != "$CUR_DENO" ]] && echo "        deno_core: manifest=${m_deno:-missing} current=$CUR_DENO"
    [[ "$m_snap" != "$actual_snap" ]] && echo "        snapshot bytes do not match manifest"
    [[ "$m_snap_size" != "$actual_snap_size" ]] && echo "        snapshot size does not match manifest"
    echo "        -> regenerate: scripts/gen-snapshot.sh $arch --product-profile $PRODUCT_PROFILE --snapshot-kind $SNAPSHOT_KIND"
    stale=1
  else
    c_ok "$PRODUCT_PROFILE-$arch: fresh"
  fi
done

[[ "$stale" -eq 0 ]] || exit 1
