#!/usr/bin/env bash
# CI smoke checks — fast sanity gates that run before the full AAR build.
# Failures here abort the pipeline early, saving time and resources.
set -euo pipefail

echo "=== Smoke: Shell script syntax ==="
bash -n scripts/*.sh
bash -n scripts/ci/*.sh
echo "    OK"

echo "=== Smoke: Rust formatting ==="
(cd engine && cargo fmt --all -- --check)
echo "    OK"

echo "=== Smoke: Rust workspace compiles ==="
(cd engine && cargo check --workspace)
echo "    OK"

echo "=== Smoke: Rust clippy ==="
(cd engine && cargo clippy --workspace --all-targets -- -D warnings)
echo "    OK"

echo "=== Smoke: Rust unit tests ==="
(cd engine && cargo test --workspace --lib --doc)
echo "    OK"

echo "=== All smoke checks passed ==="
