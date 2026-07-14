#!/usr/bin/env bash
# Host-only R6 Android flavor/R8 reachability contract.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LIB="$ROOT/platforms/android/library"
GRADLE="$LIB/build.gradle"
JAVA="$LIB/src/main/java/com/migo/runtime"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

echo "[1/5] checking full/slim flavor constants"
for flavor in full slim; do
  grep -q "^[[:space:]]*$flavor[[:space:]]*{" "$GRADLE" \
    || fail "missing Gradle $flavor flavor"
done
for field in MIGO_API_SENSORS MIGO_API_MEDIA MIGO_API_CONNECTIVITY \
  MIGO_API_COMMERCE MIGO_API_SYSTEM; do
  [[ "$(grep -c "buildConfigField .*\"$field\"" "$GRADLE")" -eq 2 ]] \
    || fail "$field must be fixed by both product flavors"
done

echo "[2/5] checking common lifecycle reachability guards"
grep -q 'if (BuildConfig.MIGO_API_SENSORS)' \
  "$JAVA/internal/NativeExports.java" \
  || fail "sensor manager lifecycle is not flavor guarded"
grep -q 'if (BuildConfig.MIGO_API_CONNECTIVITY)' \
  "$JAVA/internal/NativeExports.java" \
  || fail "connectivity manager lifecycle is not flavor guarded"
grep -q 'if (BuildConfig.MIGO_API_MEDIA)' \
  "$JAVA/internal/NativeExports.java" \
  || fail "media manager lifecycle is not flavor guarded"
grep -q 'BuildConfig.MIGO_API_MEDIA ? new AudioFocusManager' \
  "$JAVA/GameSession.java" \
  || fail "slim still constructs AudioFocusManager"

echo "[3/5] rejecting broad internal keep rules"
for rules in "$LIB"/*.pro; do
  if grep -Eq '^-keep(class| interface| enum)?[[:space:]]+.*com\.migo\.runtime\.\\?\*\*' \
      "$rules"; then
    fail "$(basename "$rules") keeps the whole SDK/internal tree"
  fi
done

echo "[4/5] checking profile-exact JNI roots"
for rules in proguard-slim.pro consumer-rules-slim.pro; do
  path="$LIB/$rules"
  grep -q 'requestVsync' "$path" || fail "$rules misses core requestVsync"
  grep -q 'onCameraFrameData' "$path" \
    && fail "$rules leaks media NativeBridge methods"
  grep -q 'bluetoothOpenAdapter' "$path" \
    && fail "$rules leaks connectivity NativeExports methods"
  grep -q 'requestMidasPayment' "$path" \
    && fail "$rules leaks commerce NativeExports methods"
done
for rules in proguard-full.pro consumer-rules-full.pro; do
  path="$LIB/$rules"
  grep -q 'native <methods>' "$path" \
    || fail "$rules must retain the complete active full NativeBridge surface"
  grep -q 'public static \*\*\* \*(\.\.\.);' "$path" \
    || fail "$rules must retain the complete active full NativeExports surface"
done

echo "[5/5] compiling the exhaustive Rust-to-R8 contract"
CONTRACT_TEST="$(mktemp -t migo-r6-jni-contract.XXXXXX)"
trap 'rm -f "$CONTRACT_TEST"' EXIT
rustc --edition 2024 --test \
  -A dead-code -A unused-mut \
  "$ROOT/engine/crates/platform/android/jni/profile_contract.rs" \
  -o "$CONTRACT_TEST"
"$CONTRACT_TEST"

echo "PASS: R6 Android flavors and R8 roots are profile exact"
