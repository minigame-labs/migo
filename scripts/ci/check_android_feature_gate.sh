#!/usr/bin/env bash
# Assert that the Android release build does NOT pull the Rust image
# decoders (`image`, `zune-image`, `zune-core`) nor the `io` crate's
# `rust-image-decode` feature.  On Android we decode through
# `BitmapFactory` / `ImageDecoder` via JNI; the Rust decoders are
# desktop/dev only and would add multiple MB to the APK.
#
# Why we need a CI guard: Cargo feature unification can silently
# re-enable these whenever a new dependency happens to turn the
# feature on transitively.  A manual `Cargo.toml` review catches it
# once; this script catches every regression forever.
#
# Exit 0 on success, non-zero (with context) on violation.

set -euo pipefail

TARGET="${1:-aarch64-linux-android}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

echo "=== Android feature-gate check (target=$TARGET) ==="

# Make sure the rustup target is installed; skip silently if not
# (developer machines without Android toolchain should not fail CI
# locally — set ALLOW_SKIP=1 on CI to keep the strict behaviour).
if ! rustup target list --installed | grep -q "^${TARGET}$"; then
    if [[ "${ALLOW_SKIP:-0}" == "1" ]]; then
        echo "    target $TARGET not installed, skipping (ALLOW_SKIP=1)"
        exit 0
    fi
    echo "ERROR: rustup target '$TARGET' not installed."
    echo "       Install with: rustup target add $TARGET"
    exit 2
fi

# Resolve the feature graph for the Android target.  We inspect the
# `platform` crate (the one the AAR links) because it is the highest-
# level entry into `io`; if `rust-image-decode` ever leaks in, it
# shows up here.
cd "$ROOT/engine"

# `cargo tree -e features` prints the *unified* feature set for the
# build graph, including transitively enabled features.  `--target`
# pins the cfg so target-specific optional deps (zune-image, image)
# are visible only when actually activated.
#
# We capture the output once and grep it three ways so a regression
# reports ALL offending items, not just the first one.
FEATURES_OUT="$(mktemp)"
trap 'rm -f "$FEATURES_OUT"' EXIT

cargo tree \
    --target "$TARGET" \
    -e features \
    --package migo-platform \
    --prefix none \
    --no-default-features \
    --quiet \
    > "$FEATURES_OUT"

VIOLATIONS=0

check_forbidden() {
    local pattern="$1"
    local reason="$2"
    # `-F` fixed-string match; `-q` silent — we print our own.
    if grep -F -q "$pattern" "$FEATURES_OUT"; then
        echo "  VIOLATION: '$pattern' is in the Android feature graph"
        echo "    reason: $reason"
        grep -F -n "$pattern" "$FEATURES_OUT" | head -5 | sed 's/^/      /'
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
}

# `cargo tree` prints crate names as `crate_name vX.Y.Z`.  Match on
# the exact prefix to avoid false positives on e.g. `imageproc`.
check_forbidden "image v" \
    "the 'image' crate is forbidden on Android (size + JNI duplication)"
check_forbidden "zune-image v" \
    "the 'zune-image' crate is forbidden on Android"
check_forbidden "zune-core v" \
    "the 'zune-core' crate is forbidden on Android"
# Feature edges look like `io feature "rust-image-decode"` in
# `cargo tree -e features` output.
check_forbidden 'feature "rust-image-decode"' \
    "the 'io/rust-image-decode' feature must be off on Android"

if [[ "$VIOLATIONS" -gt 0 ]]; then
    echo ""
    echo "FAIL: $VIOLATIONS Android feature-gate violation(s)."
    echo "      Rust image decoders must stay behind"
    echo "      cfg(not(target_os = \"android\"))."
    echo "      See engine/crates/core/Cargo.toml & crates/platform/Cargo.toml."
    exit 1
fi

echo "    OK — no forbidden Rust image decoders in the Android feature graph."
