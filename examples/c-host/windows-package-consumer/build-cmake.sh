#!/usr/bin/env bash
# Build and run the Windows package consumer the idiomatic CMake way: resolve
# the staged package with find_package(migo) and link migo::migo. This proves
# the SDK's CMake package (lib/cmake/migo) works, the counterpart to build.sh's
# raw cl + migo.lib proof.
#
# Usage: build-cmake.sh [STAGED_PREFIX]   (default: dist/migo-windows-x86_64)
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
source "$REPO_ROOT/platforms/windows/spike/lib.sh"

PREFIX_UNIX="${1:-$REPO_ROOT/dist/migo-windows-x86_64}"
[[ -f "$PREFIX_UNIX/lib/cmake/migo/migo-config.cmake" ]] || { echo "[win-cmake] no CMake package at $PREFIX_UNIX/lib/cmake/migo (run scripts/build-windows-sdk.sh first)" >&2; exit 1; }
info() { echo -e "\033[0;36m[win-cmake] $*\033[0m"; }

# Stage the package + consumer sources onto a Windows local disk.
WORK_UNIX="$WIN_TMP_UNIX/pkg-consumer-cmake"
rm -rf "$WORK_UNIX"
mkdir -p "$WORK_UNIX/sdk" "$WORK_UNIX/app"
cp -r "$PREFIX_UNIX/." "$WORK_UNIX/sdk/"
cp "$SCRIPT_DIR/consumer.c" "$SCRIPT_DIR/CMakeLists.txt" "$WORK_UNIX/app/"
WORK_DOS="$(wslpath -w "$WORK_UNIX")"
VCVARS_DOS="$(find_vcvars64)"
CMAKE_BIN_DOS="$(find_windows_cmake_dir)"

BATCH="$WORK_UNIX/build-cmake.bat"
cat > "$BATCH" <<BAT
@echo off
set "ERRORLEVEL="
call "${VCVARS_DOS}" >nul
set "PATH=${CMAKE_BIN_DOS};%PATH%"
cd /d ${WORK_DOS} || exit /b 90

echo [win-cmake] configuring with find_package(migo) from the staged package
cmake -S app -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_PREFIX_PATH="${WORK_DOS}\\sdk"
set STEP=%errorlevel%
if not "%STEP%"=="0" ( echo === CONFIGURE_EXIT=%STEP% === & exit /b %STEP% )

echo [win-cmake] building
cmake --build build
set STEP=%errorlevel%
if not "%STEP%"=="0" ( echo === BUILD_EXIT=%STEP% === & exit /b %STEP% )
echo [win-cmake] BUILD OK -- migo::migo resolved include + import lib + defines

echo [win-cmake] running (migo.dll + runtime DLLs load from the package bin)
set "PATH=${WORK_DOS}\\sdk\\bin;%PATH%"
build\\consumer.exe
set RUN=%errorlevel%
echo === RUN_EXIT=%RUN% ===
exit /b %RUN%
BAT

RUN_LOG="$WORK_UNIX/build-cmake.log"
info "building + running the CMake package consumer on Windows"
run_windows_batch "$BATCH" | tee "$RUN_LOG"

if [[ ! -f "$WORK_UNIX/build/consumer.exe" ]]; then
    echo "[win-cmake] consumer.exe was not produced -- configure/build failed, see $RUN_LOG" >&2
    exit 1
fi
grep -q "migo windows package consumer: OK" "$RUN_LOG" \
    && info "RESULT: PASS -- find_package(migo) built + ran the packaged SDK" \
    || { echo "[win-cmake] built but did not print the OK line -- see $RUN_LOG" >&2; exit 2; }
