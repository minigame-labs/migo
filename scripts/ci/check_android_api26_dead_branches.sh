#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

patterns=(
  'Build\.VERSION\.SDK_INT\s*<\s*Build\.VERSION_CODES\.(O|M|N)'
  'Build\.VERSION\.SDK_INT\s*>=\s*Build\.VERSION_CODES\.(HONEYCOMB|LOLLIPOP|M|N)'
  'Build\.VERSION_CODES\.(HONEYCOMB|LOLLIPOP)\b'
)

if rg -n --pcre2 "${patterns[0]}|${patterns[1]}|${patterns[2]}" \
  platforms/android/library/src/main/java -g '*.java'
then
  echo "Found dead Android API<26 compatibility branches"
  exit 1
fi
