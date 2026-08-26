#!/usr/bin/env bash
# Run cargo-audit and the resolved license/source policy as one fail-closed gate.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENGINE="$ROOT/engine"
POLICY="$ROOT/supply-chain.toml"
AUDIT_JSON="$(mktemp)"
METADATA_JSON="$(mktemp)"
trap 'rm -f "$AUDIT_JSON" "$METADATA_JSON"' EXIT

if command -v cargo-audit >/dev/null 2>&1; then
    audit=(cargo-audit audit)
elif cargo audit --version >/dev/null 2>&1; then
    audit=(cargo audit)
else
    echo "ERROR: cargo-audit 0.22.2 is required" >&2
    exit 1
fi

version="$("${audit[@]}" --version 2>/dev/null || true)"
if [[ "$version" != *"0.22.2"* ]]; then
    echo "ERROR: cargo-audit 0.22.2 is required, found: ${version:-unknown}" >&2
    exit 1
fi

# cargo-audit returns non-zero for a finding. Preserve its JSON so the policy
# checker can print every exact package/advisory rather than replacing that with
# a generic command failure. Invalid/incomplete output still fails JSON parsing.
set +e
(cd "$ENGINE" && "${audit[@]}" --json) > "$AUDIT_JSON"
audit_status=$?
set -e
if [[ ! -s "$AUDIT_JSON" ]]; then
    echo "ERROR: cargo-audit produced no JSON (exit $audit_status)" >&2
    exit 1
fi

(cd "$ENGINE" && cargo metadata --format-version 1 --locked --offline) \
    > "$METADATA_JSON"

python3 "$ROOT/scripts/check-supply-chain.py" audit \
    --audit-json "$AUDIT_JSON" \
    --metadata-json "$METADATA_JSON" \
    --policy "$POLICY" \
    --workspace-root "$ROOT"
