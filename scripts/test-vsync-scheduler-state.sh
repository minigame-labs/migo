#!/usr/bin/env bash
set -euo pipefail

# R1: host-JVM test for the demand-driven one-shot VSync state machine.
# VsyncSchedulerState is deliberately free of android.* deps so it compiles and
# runs under a plain JDK, mirroring scripts/test-android-lifecycle-state.sh.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STATE_SRC="$ROOT_DIR/platforms/android/library/src/main/java/com/migo/runtime/internal/VsyncSchedulerState.java"
TEST_SRC="$ROOT_DIR/platforms/android/library/src/test/java/com/migo/runtime/internal/VsyncSchedulerStateTestMain.java"
OUT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/migo-vsync-state.XXXXXX")"
trap 'rm -rf "$OUT_DIR"' EXIT

javac -d "$OUT_DIR" "$STATE_SRC" "$TEST_SRC"
java -cp "$OUT_DIR" com.migo.runtime.internal.VsyncSchedulerStateTestMain
