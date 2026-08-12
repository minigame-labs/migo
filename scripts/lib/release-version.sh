#!/usr/bin/env bash
# The one reader of release/VERSION, which
# scripts/test-release-version-contract.sh holds as the single version source.
#
# This function existed four times over: identically in build-linux-sdk.sh,
# build-ohos-sdk.sh and build-windows-sdk.sh, and inline in build-android-sdk.sh.
# The version contract could only check that each script *read* something, never
# that they all read it the same way, so a copy was free to drift while every call
# site still looked correct. package-sdk.sh becoming a fifth consumer -- the
# published tarball name now carries the version -- is what forced the collapse.
#
# Intended to be sourced, not executed.

read_release_version() {
    local source="$1/release/VERSION"
    [[ -f "$source" ]] || { echo "[release-version] source missing: $source" >&2; exit 1; }
    local version
    version="$(tr -d '[:space:]' < "$source")"
    [[ -n "$version" ]] || { echo "[release-version] source is empty: $source" >&2; exit 1; }
    printf '%s' "$version"
}
