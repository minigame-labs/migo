#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN_SRC="$ROOT_DIR/platforms/android/library/src/main/java/com/migo/runtime/internal/platform/LifecycleRequestState.java"
LIFETIME_SRC="$ROOT_DIR/platforms/android/library/src/main/java/com/migo/runtime/internal/platform/ResourceLifetime.java"
TEST_SRC="$ROOT_DIR/platforms/android/library/src/test/java/com/migo/runtime/internal/platform/LifecycleRequestStateTestMain.java"
SYNC_MAIN_SRC="$ROOT_DIR/platforms/android/library/src/main/java/com/migo/runtime/internal/LifecycleStateSynchronizer.java"
SYNC_TEST_SRC="$ROOT_DIR/platforms/android/library/src/test/java/com/migo/runtime/internal/LifecycleStateSynchronizerTestMain.java"
OUT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/migo-lifecycle-state.XXXXXX")"
trap 'rm -rf "$OUT_DIR"' EXIT

javac -d "$OUT_DIR" \
    "$MAIN_SRC" "$LIFETIME_SRC" "$TEST_SRC" \
    "$SYNC_MAIN_SRC" "$SYNC_TEST_SRC"
java -cp "$OUT_DIR" com.migo.runtime.internal.platform.LifecycleRequestStateTestMain
java -cp "$OUT_DIR" com.migo.runtime.internal.LifecycleStateSynchronizerTestMain
