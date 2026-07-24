#!/usr/bin/env bash
# Regenerate the Windows rusty_v8 DLL export list.
#
# The exports are read from the symbols rusty_v8 actually defines rather than
# transcribed from binding.cc: a hand-kept list silently drifts when upstream
# adds or renames a binding, and the failure mode is a missing export that only
# shows up as an unresolved symbol in a downstream link. GNU nm reads the COFF
# archive directly, so this runs on Linux and needs no Windows round-trip.
#
# Usage: bash scripts/gen-windows-v8-def.sh [path/to/rusty_v8.lib]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LIB="${1:-$ROOT/engine/third_party/rusty_v8/x86_64-pc-windows-msvc/rusty_v8.lib}"
OUT="$ROOT/engine/third_party/v8-patches/rusty_v8-windows-exports.def"

[[ -f "$LIB" ]] || { printf 'no rusty_v8.lib at %s\n' "$LIB" >&2; exit 1; }
command -v nm >/dev/null || { printf 'nm not found\n' >&2; exit 1; }

symbols="$(nm -g --defined-only "$LIB" 2>/dev/null \
  | awk '$2 == "T" && $3 ~ /^v8__/ { print $3 }' | LC_ALL=C sort -u)"
count="$(printf '%s\n' "$symbols" | grep -c .)"
# A plausibility floor, not an exact pin: upstream may add bindings, but a jump
# to near-zero means nm read the wrong file or the filter stopped matching.
(( count >= 600 )) || { printf 'only %s exports found; refusing to write\n' "$count" >&2; exit 1; }

{
  printf '; Export surface of the Windows rusty_v8 DLL.\n'
  printf '; Generated -- do not hand-edit. Regenerate with:\n'
  printf ';   bash scripts/gen-windows-v8-def.sh\n'
  printf '; The list is derived from the symbols rusty_v8 actually defines, so it\n'
  printf '; cannot drift from binding.cc the way a hand-kept list would.\n'
  printf 'LIBRARY rusty_v8\n'
  printf 'EXPORTS\n'
  printf '%s\n' "$symbols" | sed 's/^/    /'
  # Added by the migo host-callback-registration patch, so neither is in the
  # archive the list above is read from.
  printf '    v8__register_host_callback\n'
  printf '    v8__host_callbacks_ready\n'
} > "$OUT"

printf 'wrote %s (%s exports + 2 registration entry points)\n' "$OUT" "$count"
