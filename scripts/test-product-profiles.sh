#!/usr/bin/env bash
# Host-only R6 product-profile contract gate.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENGINE="$ROOT/engine"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

tree_for() {
  local profile="$1"
  (cd "$ENGINE" && cargo tree -p platform -e features \
    --no-default-features --features "profile-$profile" --locked --offline)
}

js_features_for() {
  local profile="$1"
  (cd "$ENGINE" && cargo tree -p platform -e features -i js-runtime \
    --no-default-features --features "profile-$profile" --locked --offline)
}

echo "[1/7] resolving full and slim Cargo products"
FULL_TREE="$(tree_for full)" || fail "profile-full does not resolve"
SLIM_TREE="$(tree_for slim)" || fail "profile-slim does not resolve"
FULL_JS_FEATURES="$(js_features_for full)" || fail "profile-full JS features do not resolve"
SLIM_JS_FEATURES="$(js_features_for slim)" || fail "profile-slim JS features do not resolve"

echo "[2/7] checking security controls and optional-domain closure"
MISSING_DEFAULTS="$(
  grep -RnsE --include Cargo.toml \
    '^[^#]*path = "\.\./[^\"]+".*\}' "$ENGINE/crates" \
    | grep -v 'default-features = false' \
    || true
)"
[[ -z "$MISSING_DEFAULTS" ]] \
  || fail "internal dependencies must disable defaults:\n$MISSING_DEFAULTS"
grep -q 'js-runtime feature "v8-limits"' <<<"$SLIM_JS_FEATURES" \
  || fail "slim must retain js-runtime/v8-limits"
grep -q 'js-runtime feature "code-signing"' <<<"$SLIM_JS_FEATURES" \
  || fail "slim must retain js-runtime/code-signing"
if grep -Eq 'js-runtime feature "api-(sensors|media|connectivity|commerce|system)"' \
    <<<"$SLIM_JS_FEATURES"; then
  fail "slim leaked an optional js-runtime API group"
fi
if grep -Eq '(^|[[:space:]])(audio|cpal) v[0-9]' <<<"$SLIM_TREE"; then
  fail "slim leaked the native audio dependency graph"
fi
for feature in api-sensors api-media api-connectivity api-commerce api-system; do
  grep -q "js-runtime feature \"$feature\"" <<<"$FULL_JS_FEATURES" \
    || fail "full did not enable js-runtime/$feature"
done
grep -Eq '(^|[[:space:]])audio v[0-9]' <<<"$FULL_TREE" \
  || fail "full must retain native audio"

echo "[3/7] checking Gradle native artifacts are profile-exact"
grep -Fq 'jniLibs.srcDirs = ["../../../engine/jniLibs/full${migoNativeProfileSuffix}${migoWorkerSnapshotSuffix}"]' \
  "$ROOT/platforms/android/library/build.gradle" \
  || fail "Gradle full flavor is not pinned to its product/codegen native artifacts"
grep -Fq 'jniLibs.srcDirs = ["../../../engine/jniLibs/slim${migoNativeProfileSuffix}${migoWorkerSnapshotSuffix}"]' \
  "$ROOT/platforms/android/library/build.gradle" \
  || fail "Gradle slim flavor is not pinned to its product/codegen native artifacts"
grep -Fq "supportedMigoCodegenProfiles = ['z', '2', '3']" \
  "$ROOT/platforms/android/library/build.gradle" \
  || fail "Gradle does not bound the Q14 codegen profile"
grep -Fq "migoCodegenProfile == 'z' ? '' : \"-opt\${migoCodegenProfile}\"" \
  "$ROOT/platforms/android/library/build.gradle" \
  || fail "Gradle codegen profile does not isolate alternative native roots"
grep -q 'migoAbis' "$ROOT/platforms/android/library/build.gradle" \
  || fail "Gradle has no explicit ABI filter input"
grep -Fq 'jniLibs.srcDirs = []' "$ROOT/platforms/android/library/build.gradle" \
  || fail "Gradle main source set can still ingest mutable default JNI artifacts"
if grep -q 'src/main/jniLibs' "$ROOT/platforms/android/library/build.gradle" \
    "$ROOT/scripts/build-aar.sh" "$ROOT/scripts/build-aar.ps1"; then
  fail "mutable main jniLibs staging can cross-contaminate full/slim artifacts"
fi

echo "[4/7] checking named-profile exactness and mutual exclusion"
grep -Rqs 'all(feature = "profile-full", feature = "profile-slim")' \
  "$ENGINE/crates/platform" "$ENGINE/crates/core" "$ENGINE/crates/js-runtime" \
  || fail "named full+slim profiles are not rejected at compile time"
if (cd "$ENGINE" && cargo check -p js-runtime --no-default-features \
    --features profile-slim,api-media --locked --offline >/dev/null 2>&1); then
  fail "profile-slim must reject optional API groups added outside the named profile"
fi

echo "[5/7] running full/slim game-visible surface tests"
(cd "$ENGINE" && cargo test -j1 -p js-runtime --lib --no-default-features \
  --features profile-slim --locked --offline \
  tests_global_surface::global_surface_tests::product_profile_surface_matches_features -- --exact)
(cd "$ENGINE" && cargo test -j1 -p js-runtime --lib --no-default-features \
  --features profile-full --locked --offline \
  tests_global_surface::global_surface_tests::product_profile_surface_matches_features -- --exact)

echo "[6/7] checking build entrypoints select an exact product"
for script in "$ROOT/scripts/build-android-so.sh" "$ROOT/scripts/build-aar.sh"; do
  grep -q -- '--product-profile' "$script" \
    || fail "$(basename "$script") has no --product-profile option"
done
grep -q '^product_profile="full"$' "$ROOT/scripts/build-android-so.sh" \
  || fail "build-android-so.sh no longer defaults to the compatible full product"
grep -q '^PRODUCT_PROFILE="full"$' "$ROOT/scripts/build-aar.sh" \
  || fail "build-aar.sh no longer defaults to the compatible full product"
grep -q '\[string\]\$ProductProfile = "full"' "$ROOT/scripts/build-aar.ps1" \
  || fail "build-aar.ps1 no longer defaults to the compatible full product"
for script in "$ROOT/scripts/build-android-so.sh" "$ROOT/scripts/build-aar.sh"; do
  if invalid_output="$(bash "$script" --product-profile invalid 2>&1)"; then
    fail "$(basename "$script") accepted an invalid product profile"
  fi
  grep -q 'expected full|slim' <<<"$invalid_output" \
    || fail "$(basename "$script") did not diagnose the invalid product profile"
done
grep -q -- '--no-default-features' "$ROOT/scripts/build-android-so.sh" \
  || fail "Android Rust build does not disable dependency defaults"
grep -q 'profile-\$product_profile' "$ROOT/scripts/build-android-so.sh" \
  || fail "Android Rust build does not forward the selected Cargo profile"
grep -q -- '-PmigoAbis=' "$ROOT/scripts/build-aar.sh" \
  || fail "build-aar.sh does not forward its requested ABI set to Gradle"
grep -q -- '-PmigoAbis=' "$ROOT/scripts/build-aar.ps1" \
  || fail "build-aar.ps1 does not forward its requested ABI set to Gradle"
grep -q -- '-PmigoCodegenProfile=' "$ROOT/scripts/build-aar.sh" \
  || fail "build-aar.sh does not forward its codegen profile to Gradle"
grep -q -- '-PmigoCodegenProfile=' "$ROOT/scripts/build-aar.ps1" \
  || fail "build-aar.ps1 does not forward its codegen profile to Gradle"

echo "[7/7] checking snapshot inputs use the same named product"
grep -q 'SNAPSHOT-{product_profile}-{target_arch}' \
  "$ENGINE/crates/js-runtime/build.rs" \
  || fail "snapshot selection is not profile-qualified"

echo "PASS: R6 full/slim product contract holds"
