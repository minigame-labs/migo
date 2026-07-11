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
REG_RS="$REPO_ROOT/engine/crates/platform/android/jni/registration.rs"
EXPECTED='(IILjava/nio/ByteBuffer;IILjava/nio/ByteBuffer;IILjava/nio/ByteBuffer;IIII)V'

fail() { echo "FAIL: $1" >&2; exit 1; }

echo "[1/4] compiling library Java (compileDebugJavaWithJavac)..."
( cd "$ANDROID_DIR" && ./gradlew --quiet :library:compileDebugJavaWithJavac ) \
    || fail "compileDebugJavaWithJavac failed"

# Resolve the DEBUG javac class-root. Prefer the known debug output locations so
# a stale release build is never silently used; fall back to a deterministic
# find restricted to a debug javac path (find -print -quit avoids the SIGPIPE
# that `find | head` would raise under `set -o pipefail`).
CLASSES_DIR=""
for cand in \
    "$ANDROID_DIR/library/build/intermediates/javac/debug/classes" \
    "$ANDROID_DIR/library/build/intermediates/javac/debug/compileDebugJavaWithJavac/classes"; do
    if [ -f "$cand/com/migo/runtime/internal/NativeBridge.class" ]; then
        CLASSES_DIR="$cand"
        break
    fi
done
if [ -z "$CLASSES_DIR" ]; then
    bridge="$(find "$ANDROID_DIR/library/build" -type f -path '*javac*debug*' \
        -name NativeBridge.class -print -quit)"
    [ -n "$bridge" ] || fail "could not locate a compiled debug NativeBridge.class"
    CLASSES_DIR="${bridge%/com/migo/runtime/internal/NativeBridge.class}"
fi
echo "      classpath: $CLASSES_DIR"

# Extract the JNI descriptor of onCameraFrameData from `javap -s` output: the
# first `descriptor:` line after the method's declaration line.
descriptor_of() {
    local cls="$1"
    javap -s -classpath "$CLASSES_DIR" "com.migo.runtime.internal.$cls" \
        | awk '/[[:space:]]onCameraFrameData\(/{f=1} f&&/descriptor:/{print $2; exit}'
}

check() {
    local cls="$1" got
    got="$(descriptor_of "$cls")"
    [ -n "$got" ] || fail "$cls.onCameraFrameData not found via javap"
    [ "$got" = "$EXPECTED" ] || fail "$cls.onCameraFrameData descriptor '$got' != '$EXPECTED'"
    echo "OK:   $cls.onCameraFrameData = $got"
}

echo "[2/4] javap NativeBridge.onCameraFrameData..."
check NativeBridge
echo "[3/4] javap NativeMethods.onCameraFrameData..."
check NativeMethods

echo "[4/4] registration.rs descriptor constant..."
grep -qF "\"$EXPECTED\"" "$REG_RS" \
    || fail "registration.rs does not contain the exact descriptor \"$EXPECTED\""
echo "OK:   registration.rs pins the descriptor"

echo "PASS: camera-frame JNI descriptor contract holds (NativeBridge == NativeMethods == registration.rs)."
