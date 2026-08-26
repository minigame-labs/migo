#!/usr/bin/env bash
# Generate a CycloneDX SBOM for one concrete release artifact.
#
# The artifact already exists when this runs. Its SHA-256, source revision,
# target matrix and product profile become part of the SBOM identity, while
# `cargo metadata --locked --offline --filter-platform` limits components to
# the graph that can reach the selected shipping crate.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT=""
ARTIFACT=""
ARTIFACT_KIND=""
TARGET_LABEL=""
CARGO_TARGET=""
PROFILE=""
ROOT_PACKAGE=""
MANIFEST=""

usage() {
    cat <<'EOF'
usage: scripts/generate-sbom.sh \
  --artifact FILE --artifact-kind KIND \
  --target LABEL --cargo-target RUST-TRIPLE \
  --profile full|slim --root-package CARGO-PACKAGE \
  --manifest CARGO-TOML --out FILE
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --artifact) ARTIFACT="${2:?--artifact requires a path}"; shift 2 ;;
        --artifact=*) ARTIFACT="${1#*=}"; shift ;;
        --artifact-kind) ARTIFACT_KIND="${2:?--artifact-kind requires a value}"; shift 2 ;;
        --artifact-kind=*) ARTIFACT_KIND="${1#*=}"; shift ;;
        --target) TARGET_LABEL="${2:?--target requires a label}"; shift 2 ;;
        --target=*) TARGET_LABEL="${1#*=}"; shift ;;
        --cargo-target) CARGO_TARGET="${2:?--cargo-target requires a triple}"; shift 2 ;;
        --cargo-target=*) CARGO_TARGET="${1#*=}"; shift ;;
        --profile) PROFILE="${2:?--profile requires full or slim}"; shift 2 ;;
        --profile=*) PROFILE="${1#*=}"; shift ;;
        --root-package) ROOT_PACKAGE="${2:?--root-package requires a name}"; shift 2 ;;
        --root-package=*) ROOT_PACKAGE="${1#*=}"; shift ;;
        --manifest) MANIFEST="${2:?--manifest requires a path}"; shift 2 ;;
        --manifest=*) MANIFEST="${1#*=}"; shift ;;
        --out) OUT="${2:?--out requires a path}"; shift 2 ;;
        --out=*) OUT="${1#*=}"; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

for value in ARTIFACT ARTIFACT_KIND TARGET_LABEL CARGO_TARGET PROFILE ROOT_PACKAGE MANIFEST OUT; do
    if [[ -z "${!value}" ]]; then
        echo "ERROR: missing required ${value,,}" >&2
        usage >&2
        exit 2
    fi
done
if [[ "$PROFILE" != "full" && "$PROFILE" != "slim" ]]; then
    echo "ERROR: --profile must be full or slim" >&2
    exit 2
fi

case "$ARTIFACT" in /*) ;; *) ARTIFACT="$ROOT/$ARTIFACT" ;; esac
case "$MANIFEST" in /*) ;; *) MANIFEST="$ROOT/$MANIFEST" ;; esac
case "$OUT" in /*) ;; *) OUT="$ROOT/$OUT" ;; esac

if [[ ! -f "$ARTIFACT" ]]; then
    echo "ERROR: artifact does not exist: $ARTIFACT" >&2
    exit 1
fi
if [[ ! -f "$MANIFEST" ]]; then
    echo "ERROR: Cargo manifest does not exist: $MANIFEST" >&2
    exit 1
fi

metadata="$(mktemp)"
trap 'rm -f "$metadata"' EXIT

cargo metadata \
    --manifest-path "$MANIFEST" \
    --format-version 1 \
    --locked \
    --offline \
    --filter-platform "$CARGO_TARGET" \
    --no-default-features \
    --features "profile-$PROFILE" \
    > "$metadata"

python3 "$ROOT/scripts/generate-sbom.py" \
    --metadata "$metadata" \
    --artifact "$ARTIFACT" \
    --artifact-kind "$ARTIFACT_KIND" \
    --target "$TARGET_LABEL" \
    --profile "$PROFILE" \
    --root-package "$ROOT_PACKAGE" \
    --policy "$ROOT/supply-chain.toml" \
    --workspace-root "$ROOT" \
    --out "$OUT"
