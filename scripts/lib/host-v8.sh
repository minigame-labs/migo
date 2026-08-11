#!/usr/bin/env bash
# Resolve the linux-gnu host V8 archive and its binding.
#
# Two layouts are both legitimate and the difference is not the caller's
# business: `scripts/fetch-v8-archives.sh x86_64-linux-gnu` puts the archive and
# the binding side by side under `engine/third_party/rusty_v8/x86_64-linux-gnu`,
# while a source build of rusty_v8 leaves the archive under `<gn_out>/obj`.
#
# This exists because every script that builds the engine natively resolved it
# alone, all of them defaulting to a sibling `rusty_v8_src` checkout — so on a
# machine that used the fetch instead, the probe `verify-change.sh` uses to decide
# whether the Skia-linked crates can be built failed, and the whole run reported
# "native toolchain unavailable" and fell back to bare cargo. The visible symptom
# was not "you have no V8": it was `migo-graphics`, `migo-core`, `migo-capi` and
# `migo-platform` reporting their environment rather than the change under test,
# which is the one state a verifier must never be in quietly.
#
# Sets HOST_V8_ARCHIVE and HOST_V8_BINDING; the caller exports what it needs.

# shellcheck shell=bash

_host_v8_err() { echo -e "\033[0;31m[host-v8] $*\033[0m" >&2; }

# host_v8_resolve <repo-root>
#
# MIGO_HOST_V8_ARCHIVE and MIGO_HOST_V8_BINDING name the two artefacts directly
# and either may be set alone; they are honoured before any layout is searched,
# because a machine with neither layout is exactly where a caller has to say
# where the files are. They are resolved *here* rather than applied by the caller
# afterwards, so they go through the same existence and pointer-file checks a
# discovered pair does -- and so that naming both does not still require a layout
# to exist.
#
# Then MIGO_HOST_V8_DIR, accepting either layout under it, then the in-repo
# fetch, then a sibling rusty_v8_src build.
host_v8_resolve() {
    local repo_root="$1"
    HOST_V8_ARCHIVE="${MIGO_HOST_V8_ARCHIVE:-}"
    HOST_V8_BINDING="${MIGO_HOST_V8_BINDING:-}"

    local candidates=()
    if [[ -z "$HOST_V8_ARCHIVE" || -z "$HOST_V8_BINDING" ]]; then
        if [[ -n "${MIGO_HOST_V8_DIR:-}" ]]; then
            # Both layouts are accepted under it: pointing this at a directory
            # holding the pair and having it demand `obj/` underneath is the trap
            # this whole file exists to remove.
            candidates+=("$MIGO_HOST_V8_DIR/obj:$MIGO_HOST_V8_DIR")
            candidates+=("$MIGO_HOST_V8_DIR:$MIGO_HOST_V8_DIR")
        else
            # Preferred: the fetch verifies the archive against the manifest
            # committed beside it, so this path carries provenance a local build
            # does not.
            local fetched="$repo_root/engine/third_party/rusty_v8/x86_64-linux-gnu"
            candidates+=("$fetched:$fetched")
            local built="$repo_root/../rusty_v8_src/target/x86_64-unknown-linux-gnu/release/gn_out"
            candidates+=("$built/obj:$built")
        fi

        local candidate archive_dir binding_dir
        for candidate in "${candidates[@]}"; do
            archive_dir="${candidate%%:*}"
            binding_dir="${candidate##*:}"
            [[ -n "$HOST_V8_ARCHIVE" || -f "$archive_dir/librusty_v8.a" ]] || continue
            [[ -n "$HOST_V8_BINDING" || -f "$binding_dir/src_binding.rs" ]] || continue
            [[ -n "$HOST_V8_ARCHIVE" ]] || HOST_V8_ARCHIVE="$archive_dir/librusty_v8.a"
            [[ -n "$HOST_V8_BINDING" ]] || HOST_V8_BINDING="$binding_dir/src_binding.rs"
            break
        done
    fi

    if [[ ! -f "$HOST_V8_ARCHIVE" || ! -f "$HOST_V8_BINDING" ]]; then
        _host_v8_err "linux-gnu V8 not found. Looked for librusty_v8.a + src_binding.rs in:"
        local candidate
        for candidate in "${candidates[@]}"; do
            _host_v8_err "  ${candidate%%:*}"
        done
        [[ -n "${MIGO_HOST_V8_ARCHIVE:-}" ]] && _host_v8_err "  archive override: $MIGO_HOST_V8_ARCHIVE"
        [[ -n "${MIGO_HOST_V8_BINDING:-}" ]] && _host_v8_err "  binding override: $MIGO_HOST_V8_BINDING"
        _host_v8_err "Fetch it with: bash scripts/fetch-v8-archives.sh x86_64-linux-gnu"
        _host_v8_err "or set MIGO_HOST_V8_DIR to a directory holding the pair."
        return 1
    fi

    # An LFS pointer is a file of the right name and the wrong size, and it fails
    # later as an inscrutable link error. `stat -c %s` and not a symlink-following
    # check on purpose: a symlink reports its own size and would read as a pointer.
    if [[ "$(stat -c %s "$HOST_V8_ARCHIVE")" -le 1000000 ]]; then
        _host_v8_err "V8 archive looks like an LFS pointer: $HOST_V8_ARCHIVE"
        return 1
    fi
    return 0
}
