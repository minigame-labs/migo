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
#
# Second rule: within `## [Unreleased]` and the current `## v<version>` section,
# the `### ` category headings are a subsequence of Keep a Changelog's fixed
# order (Added, Changed, Deprecated, Removed, Fixed, Security) with no repeats.
# The v0.9.4 section that #145 assembled PR-by-PR had `### Added` three times,
# `### Fixed` three times and a paragraph duplicated inside one bullet, because
# nothing checked that a section was collated rather than concatenated.
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

# Category headings within the two Keep-a-Changelog sections: canonical order,
# no repeats.
python3 - "$changelog" "$version" <<'PY'
import re
import sys

changelog, version = sys.argv[1], sys.argv[2]
lines = open(changelog, encoding="utf-8").read().splitlines()

ORDER = ["Added", "Changed", "Deprecated", "Removed", "Fixed", "Security"]
rank = {name: i for i, name in enumerate(ORDER)}

targets = ("## [Unreleased]", f"## v{version} ")
errors = []
section = None
seen = []
last_rank = -1


def close(section, seen):
    pass


for raw in lines + ["## __eof__"]:
    if raw.startswith("## "):
        section = raw if (raw.startswith(targets[0]) or raw.startswith(targets[1]) or raw.rstrip() == f"## v{version}") else None
        seen = []
        last_rank = -1
        continue
    if section and raw.startswith("### "):
        cat = raw[4:].strip()
        if cat not in rank:
            errors.append(f"{section!r}: unknown category heading '### {cat}' (expected one of {ORDER})")
            continue
        if cat in seen:
            errors.append(f"{section!r}: '### {cat}' appears more than once -- collate its bullets under a single heading")
            continue
        if rank[cat] < last_rank:
            errors.append(
                f"{section!r}: '### {cat}' is out of order -- Keep a Changelog order is {ORDER}"
            )
        seen.append(cat)
        last_rank = max(last_rank, rank[cat])

if errors:
    for e in errors:
        print(f"ERROR: {e}", file=sys.stderr)
    print(f"CHANGELOG category-heading contract: FAIL ({len(errors)} violation(s))", file=sys.stderr)
    sys.exit(1)
PY

echo "CHANGELOG release-section contract: PASS (## v${version} present, ## [Unreleased] staged above it, category headings collated and ordered)"
