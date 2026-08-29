#!/usr/bin/env bash
# CHANGELOG.md must carry a section for the current release version.
#
# `release/VERSION` is bumped, the tag is cut, the artifacts are published --
# and nothing made the changelog keep up. v0.9.4 shipped with no `## v0.9.4`
# section at all: the entries that would have been in it sat under
# `## [Unreleased]` and were never promoted, and 58 PRs' worth of changes
# (including the removal of the `wx` namespace) had no release note anywhere. A
# reader asking "what changed in v0.9.4" found the answer was v0.9.3's.
#
# The rule: whatever `release/VERSION` says, `CHANGELOG.md` has a matching
# `## v<version>` heading. `## [Unreleased]` is the staging area above it and is
# not a substitute -- promoting it to the version heading is the release step
# this gate exists to force. Older sections may use other heading shapes
# (`## Engine — v0.9.0`, `## Linux SDK — ...`); only the current version is
# held to `## v<version>`.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(tr -d '[:space:]' < "$ROOT_DIR/release/VERSION")"
changelog="$ROOT_DIR/CHANGELOG.md"

if [[ -z "$version" ]]; then
    echo "ERROR: release/VERSION is empty" >&2
    exit 1
fi

if ! grep -qE "^## v${version//./\\.}( |\$)" "$changelog"; then
    echo "ERROR: CHANGELOG.md has no '## v${version}' section." >&2
    echo "  release/VERSION is ${version}. Promote '## [Unreleased]' to" >&2
    echo "  '## v${version} (<date>)' and open a fresh '## [Unreleased]' above it." >&2
    exit 1
fi

if ! grep -qE '^## \[Unreleased\]' "$changelog"; then
    echo "ERROR: CHANGELOG.md has no '## [Unreleased]' section for the next cycle's notes." >&2
    exit 1
fi

# [Unreleased] must sit above the version section, not below it.
unreleased_line="$(grep -nE '^## \[Unreleased\]' "$changelog" | head -1 | cut -d: -f1)"
version_line="$(grep -nE "^## v${version//./\\.}( |\$)" "$changelog" | head -1 | cut -d: -f1)"
if (( unreleased_line >= version_line )); then
    echo "ERROR: '## [Unreleased]' (line $unreleased_line) must come before '## v${version}' (line $version_line)." >&2
    exit 1
fi

echo "CHANGELOG release-section contract: PASS (## v${version} present, ## [Unreleased] staged above it)"
