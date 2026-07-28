#!/usr/bin/env bash
# Build and run a third-party MSVC consumer against the staged Windows SDK
# package, proving the package is linkable and (unlike the Android consumer)
# runnable: migo.lib resolves the migo_* surface, migo.dll loads, the ABI runs.
#
# Usage: build.sh [STAGED_PREFIX]   (default: dist/migo-windows-x86_64)
#
# Source stays in WSL; the compile+link+run happens on a Windows local disk for
# the same reason the rest of the Windows build does (UNC is unusable for the
# toolchain). The staged package and this consumer are copied there first.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
source "$REPO_ROOT/platforms/windows/spike/lib.sh"

PREFIX_UNIX="${1:-$REPO_ROOT/dist/migo-windows-x86_64}"
[[ -f "$PREFIX_UNIX/lib/migo.lib" ]] || { echo "[win-consumer] no staged package at $PREFIX_UNIX (run scripts/build-windows-sdk.sh first)" >&2; exit 1; }
[[ -f "$PREFIX_UNIX/include/migo/migo.h" ]] || { echo "[win-consumer] staged package missing headers" >&2; exit 1; }
info() { echo -e "\033[0;36m[win-consumer] $*\033[0m"; }

# Stage the package + this consumer onto a Windows local disk.
WORK_UNIX="$WIN_TMP_UNIX/pkg-consumer"
rm -rf "$WORK_UNIX"
mkdir -p "$WORK_UNIX/sdk"
cp -r "$PREFIX_UNIX/." "$WORK_UNIX/sdk/"
cp "$SCRIPT_DIR/consumer.c" "$WORK_UNIX/consumer.c"
WORK_DOS="$(wslpath -w "$WORK_UNIX")"
VCVARS_DOS="$(find_vcvars64)"

BATCH="$WORK_UNIX/build-consumer.bat"
cat > "$BATCH" <<BAT
@echo off
set "ERRORLEVEL="
call "${VCVARS_DOS}" >nul
cd /d ${WORK_DOS} || exit /b 90

echo [win-consumer] compiling + linking against the staged package (public headers + migo.lib)
rem /std:c11 is required, not a nicety: the headers assert layout with C11
rem _Static_assert, and MSVC's default C dialect (C89) does not know it. GCC and
rem Clang default to gnu11 so Linux/Android consumers never hit this; a Windows
rem consumer of a modern C ABI header opts into C11, matching the MSVC ABI lane.
cl /nologo /std:c11 /W3 /I "${WORK_DOS}\\sdk\\include" consumer.c /Fe:consumer.exe /link /LIBPATH:"${WORK_DOS}\\sdk\\lib" migo.lib
set STEP=%errorlevel%
if not "%STEP%"=="0" ( echo === COMPILE_LINK_EXIT=%STEP% === & exit /b %STEP% )
echo [win-consumer] LINK OK -- migo.lib resolved the migo_* surface, no C++ runtime conflict

echo [win-consumer] running (migo.dll + runtime DLLs load from the package bin)
set "PATH=${WORK_DOS}\\sdk\\bin;%PATH%"
consumer.exe
set RUN=%errorlevel%
echo === RUN_EXIT=%RUN% ===
exit /b %RUN%
BAT

RUN_LOG="$WORK_UNIX/build-consumer.log"
info "building + running the package consumer on Windows"
run_windows_batch "$BATCH" | tee "$RUN_LOG"

if [[ ! -f "$WORK_UNIX/consumer.exe" ]]; then
    echo "[win-consumer] consumer.exe was not produced -- link failed, see $RUN_LOG" >&2
    exit 1
fi
info "consumer.exe built ($(stat -c %s "$WORK_UNIX/consumer.exe") bytes)"
grep -q "migo windows package consumer: OK" "$RUN_LOG" \
    && info "RESULT: PASS -- external MSVC consumer linked + ran the packaged SDK" \
    || { echo "[win-consumer] consumer linked but did not print the OK line (ran but ABI call path differed) -- see $RUN_LOG" >&2; exit 2; }
