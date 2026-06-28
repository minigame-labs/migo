#!/usr/bin/env bash
# Shared "snapshot input fingerprint" helpers — sourced by gen-snapshot.sh and
# check-snapshot-freshness.sh so both compute identical values.
#
# The fingerprint captures the inputs that make a snapshot valid/stale:
#   * js_sources_sha256  — every tracked extension *.js under js-runtime. A
#       snapshot bakes the post-execution heap of these, and with a snapshot the
#       extension JS is NOT re-loaded from source at runtime, so changing JS
#       without regenerating silently runs the OLD code.
#   * deno_core_version  — deno_core (hence V8) version; a bump changes builtins
#       + the external-reference table, breaking the snapshot's V8 magic number.
#
# NOT covered by this v1 fingerprint (rely on dev discipline + on-device smoke
# test): pure op rename keeping the same op count with no JS change; rebuilding
# the android V8 archive with different GN flags.

# sha256 of all tracked extension JS under js-runtime (deterministic, sorted).
# $1 = repo root
snapshot_js_hash() {
  local root="$1"
  ( cd "$root" && \
    git ls-files 'engine/crates/js-runtime/*.js' 'engine/crates/js-runtime/**/*.js' \
      | LC_ALL=C sort \
      | xargs sha256sum \
      | sha256sum | awk '{print $1}' )
}

# deno_core version from Cargo.lock. $1 = engine dir
snapshot_deno_core_version() {
  local engine="$1"
  awk '/^name = "deno_core"$/{getline; gsub(/[",]/,"",$3); print $3; exit}' "$engine/Cargo.lock"
}
