#!/usr/bin/env bash
set -euo pipefail
# Run one `cargo check` for a package on the Windows toolchain.
#
# Usage: probe-layer.sh <package>
#
# The generated batch carries every environment fact the spike had to discover
# the hard way. An earlier version set only PATH and CARGO_TARGET_DIR, which
# meant the script could not reproduce the results its own report claimed --
# anything past `migo-capi-abi` failed. Each block below cites what it is for.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

if [[ $# -lt 1 ]]; then
    echo "usage: probe-layer.sh <package>" >&2
    exit 2
fi
PACKAGE="$1"

# $PACKAGE is interpolated unquoted into the generated batch and also becomes
# part of its filename: an unvalidated value containing e.g. `&` would let cmd
# run a second command whose exit status replaces cargo's, and one containing
# `/` or spaces would break the batch path.
if [[ ! "$PACKAGE" =~ ^[A-Za-z0-9_.-]+$ ]]; then
    echo "error: invalid package name '$PACKAGE' (must match ^[A-Za-z0-9_.-]+\$)" >&2
    exit 2
fi

require_synced_worktree
WORKTREE_SHA="$(git -C "$WIN_WORKTREE_UNIX" rev-parse --short HEAD)"

VCVARS_DOS="$(find_vcvars64)"

# Optional; only the V8-dependent layers need it. Left unset, rusty_v8's build
# script tries to download the archive itself, which on this machine ran at
# ~4 KB/s and stalled.
V8_ARCHIVE_LINE=""
if [[ -n "${MIGO_WIN_V8_ARCHIVE:-}" ]]; then
    V8_ARCHIVE_LINE="set RUSTY_V8_ARCHIVE=${MIGO_WIN_V8_ARCHIVE}"
fi

# Optional. Skia's git-sync-deps reaches chromium.googlesource.com, which is
# unreachable from Windows on this machine while WSL reaches it fine -- the
# proxy lives on the Windows side but cmd.exe does not inherit it.
PROXY_LINES=""
if [[ -n "${MIGO_WIN_PROXY:-}" ]]; then
    PROXY_LINES="set HTTPS_PROXY=${MIGO_WIN_PROXY}
set HTTP_PROXY=${MIGO_WIN_PROXY}"
fi

mkdir -p "$WIN_TMP_UNIX"
BATCH="$WIN_TMP_UNIX/probe-$PACKAGE.bat"
cat > "$BATCH" <<BAT
@echo off
rem An inherited variable of this name makes %errorlevel% expand to it forever.
set "ERRORLEVEL="

rem bindgen runs libclang, which reads INCLUDE -- not the -imsvc flags gn hands
rem the Skia build. Without a developer environment it cannot find <cassert>.
call "${VCVARS_DOS}" >nul

rem A whitelist, not a reordering. An Android NDK ships its own clang-cl and
rem clang resource directory; putting LLVM first is NOT enough, because the
rem resolution inside skia-bindings' bindgen does not consult PATH order. The
rem NDK directory has to be absent. Symptoms when it is not: MSVC's STL reports
rem "STL1000: expected Clang 19.0.0 or newer", or bindgen drowns in undeclared
rem __builtin_ia32_* from clang 12's intrinsics headers.
set "PATH=${MIGO_WIN_LLVM_DIR_DOS}\\bin;%USERPROFILE%\\.cargo\\bin;${WIN_TOOLS_DOS};%SystemRoot%\\system32;%SystemRoot%;%SystemRoot%\\System32\\Wbem;${WIN_EXTRA_PATH_DOS}"

rem ninja calls the ANSI GetFullPathNameA, capped at MAX_PATH regardless of
rem LongPathsEnabled. The default registry path blows past it during the Skia
rem build, so both roots are kept short.
set CARGO_HOME=${WIN_CARGO_HOME_DOS}
set CARGO_TARGET_DIR=${WIN_TARGET_DOS}

rem Skia's egl feature needs EGL/KHR headers. Windows has no system EGL -- that
rem is what ANGLE supplies -- so the Khronos registry headers are staged the
rem same way scripts/dev-setup-skia.sh does it on Linux.
set INCLUDE=${WIN_HEADERS_DOS};%INCLUDE%
${PROXY_LINES}
${V8_ARCHIVE_LINE}

cd /d ${WIN_WORKTREE_DOS}\\engine || exit /b 90
echo [probe] package=${PACKAGE} target=${WIN_TARGET_TRIPLE} sha=${WORKTREE_SHA}
cargo check -p ${PACKAGE} --target ${WIN_TARGET_TRIPLE} 2>&1
rem Capture before echoing: the echo itself succeeds and would otherwise
rem become the batch file's exit status, making every probe look like a pass.
rem (A rem line is inert here; only real commands reset %errorlevel%. Note the
rem absence of backticks: this heredoc is unquoted, so a backticked word would
rem be run by bash as a command substitution rather than written to the file.)
set CARGO_EXIT=%errorlevel%
echo === EXIT=%CARGO_EXIT% ===
exit /b %CARGO_EXIT%
BAT

run_windows_batch "$BATCH"
