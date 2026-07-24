#!/usr/bin/env bash
set -euo pipefail
# Build, link and RUN the headless player on the Windows MSVC toolchain, and
# bring its captured frame back for inspection.
#
# Usage: run-player.sh <windows-game-dir-dos> [seconds]
#
# This is the step past run-tests.sh. Passing tests prove the code executes;
# they do not prove the graphics stack can reach a GPU. On Windows that means
# ANGLE: EGL is not a system library there, so a context has to come from the
# ANGLE DLLs, and nothing about a unit test exercises that. The player renders a
# real game offscreen and reads the presented frame back as a PNG, which is the
# same evidence the Linux port used -- an image with content, not a green log.
#
# The ANGLE runtime DLLs must be on MIGO_WIN_EXTRA_PATH; they are loaded by name
# at run time, exactly as the engine loads them on a user's machine.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

if [[ $# -lt 1 ]]; then
    echo "usage: run-player.sh <windows-game-dir-dos> [seconds]" >&2
    exit 2
fi
GAME_DIR_DOS="$1"; shift
SECS="${1:-6}"
if [[ ! "$SECS" =~ ^[0-9]+$ ]]; then
    echo "error: seconds must be a number" >&2
    exit 2
fi
# Interpolated unquoted into a generated batch, so keep it to a tame path class
# for the same reason run-tests.sh validates its package name.
if [[ ! "$GAME_DIR_DOS" =~ ^[A-Za-z]:[A-Za-z0-9_.:\\-]+$ ]]; then
    echo "error: game dir must be a plain DOS path (got '$GAME_DIR_DOS')" >&2
    exit 2
fi

if [[ -z "${MIGO_WIN_V8_ARCHIVE:-}" ]]; then
    echo "error: MIGO_WIN_V8_ARCHIVE must point at a Windows rusty_v8 import library" >&2
    exit 94
fi

require_synced_worktree
WORKTREE_SHA="$(git -C "$WIN_WORKTREE_UNIX" rev-parse --short HEAD)"
VCVARS_DOS="$(find_vcvars64)"

BINDING_LINE=""
if [[ -n "${MIGO_WIN_V8_SRC_BINDING:-}" ]]; then
    BINDING_LINE="set RUSTY_V8_SRC_BINDING_PATH=${MIGO_WIN_V8_SRC_BINDING}"
fi
PROXY_LINES=""
[[ -n "${MIGO_WIN_PROXY:-}" ]] && PROXY_LINES="set HTTPS_PROXY=${MIGO_WIN_PROXY}
set HTTP_PROXY=${MIGO_WIN_PROXY}"

PNG_DOS="${MIGO_WIN_PLAYER_PNG:-C:\\migo-win-spike-tmp\\player-frame.png}"
PNG_UNIX="$(win_to_unix_path "$PNG_DOS")"
rm -f "$PNG_UNIX"

mkdir -p "$WIN_TMP_UNIX"
BATCH="$WIN_TMP_UNIX/runplayer.bat"
cat > "$BATCH" <<BAT
@echo off
rem An inherited variable of this name makes %errorlevel% expand to it forever.
set "ERRORLEVEL="
call "${VCVARS_DOS}" >nul

rem Whitelist, not a reordering: an Android NDK on PATH shadows clang-cl and the
rem clang resource dir, and bindgen does not consult PATH order. MIGO_WIN_EXTRA_PATH
rem is where the ANGLE DLLs and the V8 DLL come in -- both are loaded by name at
rem run time, so they must be findable by the process, not just by the linker.
set "PATH=${MIGO_WIN_LLVM_DIR_DOS}\\bin;%USERPROFILE%\\.cargo\\bin;${WIN_TOOLS_DOS};%SystemRoot%\\system32;%SystemRoot%;%SystemRoot%\\System32\\Wbem;${WIN_EXTRA_PATH_DOS}"

set CARGO_HOME=${WIN_CARGO_HOME_DOS}
set CARGO_TARGET_DIR=${WIN_TARGET_DOS}
set INCLUDE=${WIN_HEADERS_DOS};%INCLUDE%
set RUSTY_V8_ARCHIVE=${MIGO_WIN_V8_ARCHIVE}
set MIGO_PLAYER_PNG=${PNG_DOS}
${BINDING_LINE}
${PROXY_LINES}

cd /d ${WIN_WORKTREE_DOS}\\engine || exit /b 90
echo [player] game=${GAME_DIR_DOS} secs=${SECS} sha=${WORKTREE_SHA}
cargo run -p migo-player --target ${WIN_TARGET_TRIPLE} -- ${GAME_DIR_DOS} ${SECS} 2>&1
rem Capture before echoing: echo succeeds and would otherwise become the exit
rem status, turning a crashed player into a pass.
set RUN_EXIT=%errorlevel%
echo === EXIT=%RUN_EXIT% ===
exit /b %RUN_EXIT%
BAT

run_windows_batch "$BATCH"

if [[ ! -f "$PNG_UNIX" ]]; then
    echo "error: player exited 0 but wrote no frame to $PNG_DOS" >&2
    echo "       a run that presents nothing is not a rendering run" >&2
    exit 1
fi

size="$(stat -c%s "$PNG_UNIX")"
printf '[player] captured frame: %s (%s bytes)\n' "$PNG_DOS" "$size"
