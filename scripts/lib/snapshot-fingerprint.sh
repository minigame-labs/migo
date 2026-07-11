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
# NOT covered by this v1 fingerprint (rely on dev discipline + on-device smoke
# test): pure op rename keeping the same op count with no JS change; rebuilding
# the android V8 archive with different GN flags.

# sha256 of every extension *.js on disk under js-runtime.
#
# This MUST stay byte-identical to the Rust helper
# engine/crates/js-runtime/build_snapshot.rs (shared by build.rs and its
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
    find engine/crates/js-runtime -type f -name '*.js' \
      | LC_ALL=C sort \
      | xargs -r -d '\n' sha256sum \
      | sha256sum | awk '{print $1}' )
}

# deno_core version from Cargo.lock. $1 = engine dir
snapshot_deno_core_version() {
  local engine="$1"
  awk '/^name = "deno_core"$/{getline; gsub(/[",]/,"",$3); print $3; exit}' "$engine/Cargo.lock"
}
