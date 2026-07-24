#!/usr/bin/env bash
# ============================================================
# Prove the Windows rusty_v8 DLL is usable, not merely linkable.
# Location: scripts/test-windows-v8-dll.sh
#
# scripts/build-v8-windows.sh producing a .dll and an import library only shows
# the link succeeded. This links a plain C consumer against that import library
# and runs it, so a DLL that builds but cannot load -- a missing dependency, a
# botched export table, an entry point that faults -- fails here instead of much
# later inside a Rust test binary where the cause is far from the symptom.
#
# The probe is C on purpose: the export surface is C, and a C consumer proves it
# without dragging in Rust, cargo or any V8 header.
#
# Usage: bash scripts/test-windows-v8-dll.sh
# ============================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET="x86_64-pc-windows-msvc"
ART_DIR="$PROJECT_ROOT/engine/third_party/rusty_v8/$TARGET"

WORK_WIN="${MIGO_WIN_DLL_PROBE_DIR:-C:\\migo-win-spike-tmp\\v8dllprobe}"
win_to_unix() { printf '/mnt/%s' "$(printf '%s' "$1" | sed 's|\\|/|g; s|^\(.\):|\L\1|')"; }
WORK_UNIX="$(win_to_unix "$WORK_WIN")"

info() { printf '\033[0;36m[v8-dll] %s\033[0m\n' "$*"; }
ok()   { printf '\033[0;32m[v8-dll] %s\033[0m\n' "$*"; }
err()  { printf '\033[0;31m[v8-dll] %s\033[0m\n' "$*" >&2; }

DLL="$ART_DIR/rusty_v8.dll"
IMPLIB="$ART_DIR/rusty_v8.dll.lib"
for f in "$DLL" "$IMPLIB"; do
    [[ -f "$f" ]] || { err "missing $f -- run scripts/build-v8-windows.sh first"; exit 1; }
done

VSWHERE="/mnt/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
[[ -f "$VSWHERE" ]] || { err "vswhere not found -- install VS Build Tools with the C++ workload"; exit 1; }
VS_ROOT="$("$VSWHERE" -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 \
    -latest -property installationPath | tr -d '\r')"
[[ -n "$VS_ROOT" ]] || { err "no Visual Studio with the C++ workload"; exit 1; }

mkdir -p "$WORK_UNIX"
cp "$DLL" "$WORK_UNIX/rusty_v8.dll"
cp "$IMPLIB" "$WORK_UNIX/rusty_v8.dll.lib"

# Declared locally rather than included: this is exactly what a third-party C
# consumer of the DLL has to be able to do.
cat > "$WORK_UNIX/probe.c" <<'PROBE'
#include <stdio.h>

extern const char* v8__V8__GetVersion(void);

int main(void) {
    const char* version = v8__V8__GetVersion();
    if (version == NULL || version[0] == '\0') {
        printf("PROBE-FAIL empty version\n");
        return 1;
    }
    printf("PROBE-OK %s\n", version);
    return 0;
}
PROBE

cat > "$WORK_UNIX/probe.bat" <<BAT
@echo off
setlocal
set "ERRORLEVEL="
call "${VS_ROOT}\\VC\\Auxiliary\\Build\\vcvars64.bat" >nul
cd /d "${WORK_WIN}"
cl /nologo /W4 /WX probe.c rusty_v8.dll.lib /Fe:probe.exe >compile.log 2>&1
rem A bare cl failure must not be masked: capture its code before anything else
rem runs, because the next command replaces %errorlevel%.
set CL_EXIT=%errorlevel%
if not "%CL_EXIT%"=="0" (
  type compile.log
  exit /b %CL_EXIT%
)
probe.exe >run.log 2>&1
set RUN_EXIT=%errorlevel%
type run.log
exit /b %RUN_EXIT%
BAT

info "linking and running a C consumer against rusty_v8.dll"
if ! ( cd "$WORK_UNIX" && cmd.exe /c "probe.bat" ); then
    err "probe failed -- the DLL links but does not load or run"
    [[ -f "$WORK_UNIX/compile.log" ]] && sed -n '1,20p' "$WORK_UNIX/compile.log" >&2
    exit 1
fi

RESULT="$(tr -d '\r' < "$WORK_UNIX/run.log")"
case "$RESULT" in
    PROBE-OK\ *) ;;
    *) err "unexpected probe output: $RESULT"; exit 1 ;;
esac

VERSION="${RESULT#PROBE-OK }"
# Shape check only. Pinning the exact version here would make a routine V8 bump
# fail in a place that has nothing to say about it; an empty or garbage string
# is what this is guarding against.
if ! printf '%s' "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.'; then
    err "version does not look like a V8 version: '$VERSION'"
    exit 1
fi

ok "C consumer linked the import library, loaded rusty_v8.dll and called into V8"
ok "v8 version reported: $VERSION"
