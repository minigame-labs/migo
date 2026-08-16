#!/usr/bin/env bash
# Content in this repository must not reference a namespace the engine does
# not install.
#
# The engine builds only `migo` (see `97_migo_namespace.js`) -- there is no
# `wx` global unless a host or game explicitly loads a platform-compat
# adapter, which none of the conformance content under tests/c_host does.
# `wx.anything` there throws ReferenceError on first use, which aborts paint
# and leaves the screen black -- with no clue in the failure that a missing
# namespace was the problem. That is exactly how it shipped once: every probe
# written for the gamepad and IME work called `wx.getGamepads()` (back when
# gamepad specifically was migo-only even while the engine still built a `wx`
# mirror for everything else), and the black screen was read as a
# frame-driving or rendering fault on the device.
#
# Scope: the conformance content under tests/c_host. The host-integration
# examples that used to live beside it moved to minigame-labs/migo-examples,
# which uses `migo.*` throughout for the same reason.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONTENT_ROOT="$ROOT_DIR/tests/c_host"

errors=0
scanned=0
while IFS= read -r -d '' source_path; do
    case "$source_path" in
        */build/*) continue ;;
    esac
    scanned=$((scanned + 1))
    # Word boundary, literal `wx.` form -- see git history for this file's
    # note on why bracket notation and aliasing are out of scope for a static
    # grep-shaped gate.
    if grep -qE '\bwx\.[A-Za-z_$]' "$source_path"; then
        relative="${source_path#"$ROOT_DIR"/}"
        echo "ERROR: $relative references \`wx.*\`, but the engine does not install a wx global -- this will throw ReferenceError at runtime. Use migo.* instead." >&2
        errors=1
    fi
done < <(find "$CONTENT_ROOT" -name "*.js" -print0)

if [[ "$scanned" -eq 0 ]]; then
    echo "ERROR: no content JS found to scan; the gate would pass vacuously" >&2
    exit 1
fi

if [[ "$errors" -ne 0 ]]; then
    exit 1
fi

echo "OK: $scanned content sources reference no undefined wx namespace"
