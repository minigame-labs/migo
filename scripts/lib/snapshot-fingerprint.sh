#!/usr/bin/env bash
# Shared "snapshot input fingerprint" helpers — sourced by gen-snapshot.sh
# (via write-snapshot-manifest.sh) and check-snapshot-freshness.sh so both
# compute identical values.
#
# The fingerprint captures the inputs that make a snapshot valid/stale:
#   * js_sources_sha256  — every extension *.js on disk under js-runtime. A
#       snapshot bakes the post-execution heap of these, and with a snapshot the
#       extension JS is NOT re-loaded from source at runtime, so changing JS
#       without regenerating silently runs the OLD code.
#   * deno_core_version  — deno_core (hence V8) version; a bump changes builtins
#       + the external-reference table, breaking the snapshot's V8 magic number.
#
# Schema v3 also binds the runtime kind (host versus Worker), fingerprints the
# named product feature set, every Rust source
# in js-runtime + snapshot-gen, both feature manifests, and Cargo.lock,
# conservatively covering op signatures/order, generator options, dependencies,
# and V8 flags.

SNAPSHOT_SCHEMA_VERSION=3

snapshot_validate_kind_profile() {
  local snapshot_kind="$1"
  local product_profile="$2"
  case "$snapshot_kind" in
    host|worker) ;;
    *)
      echo "invalid snapshot kind '$snapshot_kind' (expected host|worker)" >&2
      return 1
      ;;
  esac
  if [[ "$snapshot_kind" == "worker" && "$product_profile" != "full" ]]; then
    echo "Worker snapshot requires product profile full" >&2
    return 1
  fi
}

# Each kind/profile snapshot set is a two-ABI release unit. An absent set stays
# optional, but once either ABI exists require freshness for both so a partial
# release candidate cannot pass a default scan.
snapshot_default_arches() {
  local _snapshot_kind="$1"
  shift
  if [[ "$#" -gt 0 ]]; then
    printf '%s\n' aarch64 x86_64
  fi
}

# Hash identity of a materialized artifact or its Git LFS pointer. Freshness-only
# CI intentionally checks out without LFS; a valid pointer's oid and size are
# the identity of the materialized content, not of the pointer text.
snapshot_artifact_hash() {
  local artifact="$1"
  local size oid declared_size
  [[ -s "$artifact" ]] || {
    echo "snapshot artifact missing or empty: $artifact" >&2
    return 1
  }
  size="$(stat -c %s "$artifact")" || return 1
  if (( size >= 100000 )); then
    sha256sum "$artifact" | awk '{print $1}'
    return
  fi

  if [[ "$(sed -n '1p' "$artifact")" != \
        "version https://git-lfs.github.com/spec/v1" ]]; then
    echo "artifact is too small and is not a valid Git LFS pointer: $artifact" >&2
    return 1
  fi
  oid="$(sed -n 's/^oid sha256:\([0-9a-f]\{64\}\)$/\1/p' "$artifact")"
  declared_size="$(sed -n 's/^size \([0-9][0-9]*\)$/\1/p' "$artifact")"
  if [[ ! "$oid" =~ ^[0-9a-f]{64}$ || ! "$declared_size" =~ ^[0-9]+$ ]]; then
    echo "malformed Git LFS pointer: $artifact" >&2
    return 1
  fi
  printf '%s\n' "$oid"
}

snapshot_artifact_size() {
  local artifact="$1"
  local size declared_size
  [[ -s "$artifact" ]] || return 1
  size="$(stat -c %s "$artifact")" || return 1
  if (( size >= 100000 )); then
    printf '%s\n' "$size"
    return
  fi
  [[ "$(sed -n '1p' "$artifact")" == \
     "version https://git-lfs.github.com/spec/v1" ]] || return 1
  declared_size="$(sed -n 's/^size \([0-9][0-9]*\)$/\1/p' "$artifact")"
  [[ "$declared_size" =~ ^[0-9]+$ ]] || return 1
  printf '%s\n' "$declared_size"
}

snapshot_v8_archive_hash() {
  snapshot_artifact_hash "$1"
}

# Generation and native linking require real bytes. Unlike freshness, these
# paths must never accept an LFS pointer and write a manifest from it.
snapshot_require_materialized_artifact() {
  local artifact="$1"
  local kind="$2"
  local size
  [[ -s "$artifact" ]] || {
    echo "$kind missing or empty: $artifact" >&2
    return 1
  }
  size="$(stat -c %s "$artifact")" || return 1
  if (( size < 100000 )); then
    echo "$kind is only $size bytes (likely a Git LFS pointer): $artifact" >&2
    return 1
  fi
}

snapshot_require_materialized_v8_archive() {
  snapshot_require_materialized_artifact "$1" "V8 archive"
}

snapshot_require_materialized_snapshot() {
  snapshot_require_materialized_artifact "$1" "snapshot"
}

# sha256 of every extension *.js on disk under js-runtime.
#
# This MUST stay byte-identical to the Rust helper
# engine/crates/runtime-v8/build_snapshot.rs (shared by build.rs and its
# regression test). Both:
#   * enumerate via a filesystem walk (NOT `git ls-files`), so it works in a
#     worktree without a .git directory and untracked/generated embedded JS also
#     invalidates the snapshot;
#   * order by the raw bytes of the repo-root-relative path (LC_ALL=C) — NOT the
#     component-wise order of git pathspecs or Rust's `Path` `Ord`, which differ
#     whenever a filename byte is below `/` (0x2f), e.g. `worker.js` vs
#     `worker/x.js`; and
#   * hash `"<sha256hex>  <relpath>\n"` lines (the sha256sum column format) with
#     an outer sha256.
# $1 = repo root
snapshot_js_hash() {
  local root="$1"
  ( cd "$root" && \
    find engine/crates/runtime-v8 -type f -name '*.js' \
      | LC_ALL=C sort \
      | xargs -r -d '\n' sha256sum \
      | sha256sum | awk '{print $1}' )
}

snapshot_runtime_hash() {
  local root="$1"
  ( cd "$root" && \
    { find engine/crates/runtime-v8 engine/tools/snapshot-gen \
        -type f -name '*.rs' || exit 1; \
      printf '%s\n' \
        engine/crates/runtime-v8/Cargo.toml \
        engine/tools/snapshot-gen/Cargo.toml \
        engine/Cargo.lock || exit 1; } \
      | LC_ALL=C sort \
      | xargs -r -d '\n' sha256sum \
      | sha256sum | awk '{print $1}' )
}

snapshot_profile_features() {
  case "$1" in
    full)
      printf '%s\n' api-commerce api-connectivity api-media api-sensors \
        api-system code-signing profile-full v8-limits
      ;;
    slim)
      printf '%s\n' code-signing profile-slim v8-limits
      ;;
    *)
      echo "unsupported snapshot product profile: $1" >&2
      return 1
      ;;
  esac
}

snapshot_feature_hash() {
  snapshot_profile_features "$1" | LC_ALL=C sort | sha256sum | awk '{print $1}'
}

snapshot_profile_features_json() {
  case "$1" in
    full)
      printf '%s' '["api-commerce","api-connectivity","api-media","api-sensors","api-system","code-signing","profile-full","v8-limits"]'
      ;;
    slim)
      printf '%s' '["code-signing","profile-slim","v8-limits"]'
      ;;
    *)
      echo "unsupported snapshot product profile: $1" >&2
      return 1
      ;;
  esac
}

# deno_core version from Cargo.lock. $1 = engine dir
snapshot_deno_core_version() {
  local engine="$1"
  awk '/^name = "deno_core"$/{getline; gsub(/[",]/,"",$3); print $3; exit}' "$engine/Cargo.lock"
}
