#!/usr/bin/env bash
set -euo pipefail
# Build, link and RUN a package's tests on the Windows MSVC toolchain.
#
# Usage: run-tests.sh <package> [extra cargo args...]
#
# This is the step past `probe-layer.sh`. A `cargo check` proves the source
# type-checks; it never links and never runs, so it cannot answer the question
# that actually matters for a new platform: does the compiled code, linked
# against a real V8, execute on Windows? `cargo test` does — it codegens, links
# the test executable against `rusty_v8.lib` and the MSVC runtime, and runs it.
#
# The environment below is probe-layer.sh's, with two changes forced by the
# link+run step:
#   * The MSVC toolchain (vcvars64) is not optional here as it is for a check.
#     `cargo check` never invokes link.exe; `cargo test` does, and without the
#     developer environment the linker cannot find the CRT import libraries.
#   * A real Windows V8 archive is REQUIRED, not optional. A check can defer the
#     archive (rusty_v8 only needs it to link); a test cannot link without it.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

if [[ $# -lt 1 ]]; then
    echo "usage: run-tests.sh <package> [extra cargo args...]" >&2
    exit 2
fi
PACKAGE="$1"; shift
# Same validation as probe-layer.sh: $PACKAGE is interpolated unquoted into a
# generated batch and its filename, so an `&`/`/`/space would let cmd run a
# second command or break the batch path.
if [[ ! "$PACKAGE" =~ ^[A-Za-z0-9_.-]+$ ]]; then
    echo "error: invalid package name '$PACKAGE' (must match ^[A-Za-z0-9_.-]+\$)" >&2
    exit 2
fi

# Extra cargo args (e.g. --lib, --no-run, a test name filter). Each must be a
# tame token for the same reason $PACKAGE is validated: they land unquoted in
# the batch. Cargo flags and Rust identifiers are covered by this class.
EXTRA_ARGS=()
for a in "$@"; do
    if [[ ! "$a" =~ ^[A-Za-z0-9_.:=/-]+$ ]]; then
        echo "error: invalid cargo arg '$a' (must match ^[A-Za-z0-9_.:=/-]+\$)" >&2
        exit 2
    fi
    EXTRA_ARGS+=("$a")
done
EXTRA_ARGS_STR="${EXTRA_ARGS[*]:-}"

require_synced_worktree
WORKTREE_SHA="$(git -C "$WIN_WORKTREE_UNIX" rev-parse --short HEAD)"

VCVARS_DOS="$(find_vcvars64)"

# Required for a link. Left unset, rusty_v8's build script tries to download the
# archive itself, which on this machine ran at ~4 KB/s and stalled.
if [[ -z "${MIGO_WIN_V8_ARCHIVE:-}" ]]; then
    echo "error: MIGO_WIN_V8_ARCHIVE must point at a Windows rusty_v8 .lib -- a test links V8 and cannot defer it" >&2
    exit 94
fi

# rusty_v8 needs a bindings file matching the archive. If one is staged next to
# the archive, hand it over so the build never invokes bindgen (whose libclang
# resolution is the NDK-pollution minefield documented in the spike report).
BINDING_LINE=""
if [[ -n "${MIGO_WIN_V8_SRC_BINDING:-}" ]]; then
    BINDING_LINE="set RUSTY_V8_SRC_BINDING_PATH=${MIGO_WIN_V8_SRC_BINDING}"
fi

PROXY_LINES=""
if [[ -n "${MIGO_WIN_PROXY:-}" ]]; then
    PROXY_LINES="set HTTPS_PROXY=${MIGO_WIN_PROXY}
set HTTP_PROXY=${MIGO_WIN_PROXY}"
fi

mkdir -p "$WIN_TMP_UNIX"
BATCH="$WIN_TMP_UNIX/runtests-$PACKAGE.bat"
cat > "$BATCH" <<BAT
@echo off
rem An inherited variable of this name makes %errorlevel% expand to it forever.
set "ERRORLEVEL="

rem link.exe needs the MSVC/CRT libraries and INCLUDE; a test links, so unlike a
rem check this cannot be skipped.
call "${VCVARS_DOS}" >nul

rem A whitelist, not a reordering: an Android NDK on PATH ships its own clang-cl
rem and clang resource dir, and skia-bindings' bindgen does not consult PATH
rem order, so the NDK directory has to be ABSENT, not merely after LLVM.
set "PATH=${MIGO_WIN_LLVM_DIR_DOS}\\bin;%USERPROFILE%\\.cargo\\bin;${WIN_TOOLS_DOS};%SystemRoot%\\system32;%SystemRoot%;%SystemRoot%\\System32\\Wbem;${WIN_EXTRA_PATH_DOS}"

rem ninja calls ANSI GetFullPathNameA, capped at MAX_PATH regardless of
rem LongPathsEnabled; both roots are kept short so the Skia build stays inside it.
set CARGO_HOME=${WIN_CARGO_HOME_DOS}
set CARGO_TARGET_DIR=${WIN_TARGET_DOS}
set INCLUDE=${WIN_HEADERS_DOS};%INCLUDE%
set RUSTY_V8_ARCHIVE=${MIGO_WIN_V8_ARCHIVE}
${BINDING_LINE}
${PROXY_LINES}

cd /d ${WIN_WORKTREE_DOS}\\engine || exit /b 90
echo [test] package=${PACKAGE} target=${WIN_TARGET_TRIPLE} sha=${WORKTREE_SHA} args=${EXTRA_ARGS_STR}
cargo test -p ${PACKAGE} --target ${WIN_TARGET_TRIPLE} ${EXTRA_ARGS_STR} 2>&1
rem Capture before echoing: the echo succeeds and would otherwise become the
rem batch's exit status, making a failed test run look like a pass.
set CARGO_EXIT=%errorlevel%
echo === EXIT=%CARGO_EXIT% ===
exit /b %CARGO_EXIT%
BAT

run_windows_batch "$BATCH"
