#!/usr/bin/env bash
# Every relative link in a tracked Markdown file must point at a file that
# exists.
#
# `.github/PULL_REQUEST_TEMPLATE.md` linked `[CLA](CLA.md)` -- resolved from
# `.github/`, that is `.github/CLA.md`, which does not exist; the file is at the
# repository root. A first-time contributor, the one audience of that line,
# clicked a 404. Nothing checked it.
#
# Scope: relative links (`](path)` and `](path#anchor)`) in tracked `*.md`,
# resolved against the linking file's own directory. `http(s)://`, `mailto:`,
# bare `#anchor` fragments, and links whose target is a directory are left
# alone. Anchors within a target are not verified -- only that the target file
# is there.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

checked=0
broken=0

while IFS= read -r md; do
    dir="$(dirname "$md")"
    while IFS= read -r match; do
        link="${match#](}"
        link="${link%)}"
        case "$link" in
            http://*|https://*|mailto:*|"#"*|"") continue ;;
        esac
        target="${link%%#*}"
        [[ -n "$target" ]] || continue
        checked=$((checked + 1))
        resolved="$(cd "$dir" && realpath -m -- "$target" 2>/dev/null)" || resolved=""
        if [[ -z "$resolved" || ! -e "$resolved" ]]; then
            echo "  broken: $md -> $link" >&2
            broken=$((broken + 1))
        fi
    done < <(grep -oE '\]\([^)[:space:]]+\)' "$md" 2>/dev/null || true)
done < <(git ls-files '*.md')

if [[ "$broken" -gt 0 ]]; then
    echo "ERROR: $broken broken relative link(s) in tracked Markdown (of $checked checked)." >&2
    exit 1
fi

echo "Doc links contract: PASS ($checked relative Markdown link(s) resolve)"
