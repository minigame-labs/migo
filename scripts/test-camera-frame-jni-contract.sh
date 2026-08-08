#!/usr/bin/env bash
# ============================================================
# Q8 camera-frame JNI descriptor contract guard (host-only).
#
# Asserts that BOTH the Java forwarder (NativeMethods.onCameraFrameData) and
# the native declaration (NativeBridge.onCameraFrameData) compile to the exact
# JNI descriptor the Rust extern + RegisterNatives registration expect, and
# that registration.rs pins the same descriptor string.
#
# This never calls native code and never needs a device/emulator. It only
# checks the *descriptor* (types + arity); the function-pointer/argument-order
# binding is still validated on-device by RegisterNatives + a callback smoke.
# ============================================================
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID_DIR="$REPO_ROOT/platforms/android"
REG_RS="$REPO_ROOT/engine/crates/platform/src/android/jni/registration.rs"
PROFILE_CONTRACT_RS="$REPO_ROOT/engine/crates/platform/src/android/jni/profile_contract.rs"
EXPECTED='(IILjava/nio/ByteBuffer;IILjava/nio/ByteBuffer;IILjava/nio/ByteBuffer;IIII)V'

fail() { echo "FAIL: $1" >&2; exit 1; }

# The local verifier's Gradle mode, requested by name.
#
# `--offline` because CI has a network and no dependency cache while
# `scripts/verify-change.sh` has the cache and a network that cannot reach the module
# repositories quickly -- unconstrained, a build there stalls for tens of minutes with
# no output at all.
#
# `--no-daemon` because a verification run drives more than one Gradle build, and a
# daemon outlives its build while still holding the project lock: measured with five
# daemons alive and the owning one parked on a lock for seventeen minutes having used
# twenty seconds of CPU. Without a daemon, a build's locks die with its JVM. CI already
# takes this shape for its own Java step.
GRADLE_FLAGS=()
[[ -n "${MIGO_GRADLE_VERIFIER:-}" ]] && GRADLE_FLAGS+=(--offline --no-daemon)

echo "[1/4] compiling full/slim debug library Java..."
( cd "$ANDROID_DIR" && ./gradlew --quiet "${GRADLE_FLAGS[@]}" \
    :library:compileFullDebugJavaWithJavac \
    :library:compileSlimDebugJavaWithJavac ) \
    || fail "full/slim debug Java compilation failed"

# Extract the JNI descriptor of onCameraFrameData from `javap -s` output: the
# first `descriptor:` line after the method's declaration line.
descriptor_of() {
    local classes_dir="$1" cls="$2"
    javap -s -classpath "$classes_dir" "com.migo.runtime.internal.$cls" \
        | awk '/[[:space:]]onCameraFrameData\(/{f=1} f&&/descriptor:/{print $2; exit}'
}

check() {
    local variant="$1" classes_dir="$2" cls="$3" got
    got="$(descriptor_of "$classes_dir" "$cls")"
    [ -n "$got" ] || fail "$variant $cls.onCameraFrameData not found via javap"
    [ "$got" = "$EXPECTED" ] \
        || fail "$variant $cls.onCameraFrameData descriptor '$got' != '$EXPECTED'"
    echo "OK:   $variant $cls.onCameraFrameData = $got"
}

echo "[2/4] javap NativeBridge.onCameraFrameData for both product flavors..."
for variant in fullDebug slimDebug; do
    classes_dir="$ANDROID_DIR/library/build/intermediates/javac/$variant/classes"
    [ -f "$classes_dir/com/migo/runtime/internal/NativeBridge.class" ] \
        || fail "missing freshly compiled $variant NativeBridge.class at $classes_dir"
    check "$variant" "$classes_dir" NativeBridge
done

echo "[3/4] javap NativeMethods.onCameraFrameData for both product flavors..."
for variant in fullDebug slimDebug; do
    classes_dir="$ANDROID_DIR/library/build/intermediates/javac/$variant/classes"
    [ -f "$classes_dir/com/migo/runtime/internal/NativeMethods.class" ] \
        || fail "missing freshly compiled $variant NativeMethods.class at $classes_dir"
    check "$variant" "$classes_dir" NativeMethods
done

echo "[4/4] Rust profile contract and registration callback..."
awk -v expected="$EXPECTED" '
    /"onCameraFrameData"/ { camera = 1 }
    camera && index($0, "\"" expected "\"") { found = 1; exit }
    END { exit found ? 0 : 1 }
' "$PROFILE_CONTRACT_RS" \
    || fail "profile_contract.rs does not bind onCameraFrameData to \"$EXPECTED\""
grep -qF '"onCameraFrameData" => onCameraFrameData as *mut c_void' "$REG_RS" \
    || fail "registration.rs does not bind onCameraFrameData to its Rust callback"
grep -qF 'jni_profile_contract::active_methods(MethodDirection::JavaToNative)' "$REG_RS" \
    || fail "registration.rs no longer sources Java-to-native descriptors from profile_contract.rs"
echo "OK:   Rust profile contract pins the descriptor and registration binds the callback"

echo "PASS: camera-frame JNI descriptor contract holds (both Java flavors == Rust profile contract)."
