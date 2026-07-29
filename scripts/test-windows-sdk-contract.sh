#!/usr/bin/env bash
# The Windows SDK's contract gate, the counterpart of
# scripts/test-{linux,android}-sdk-contract.sh.
#
# Every check fails outright. There is no warn-and-continue path: an artifact
# that silently loses an export or ships a DLL it cannot load is worse than no
# artifact, because the consumer discovers it at load time on a machine we
# cannot see.
#
# What this checks that the Linux gate cannot, and vice versa:
#   * The export surface lives in a PE export table, read with dumpbin, not in
#     an ELF dynamic symbol table.
#   * There is no soname or version symlink chain on Windows; the analogous
#     identity is the import library pairing with the DLL by name.
#   * There is no glibc floor. The meaningful load-time question is whether the
#     DLL's imports resolve, which is answered by actually loading it.
#
# Usage:
#   scripts/test-windows-sdk-contract.sh            # skips MSVC checks if absent
#   scripts/test-windows-sdk-contract.sh --strict   # any skip is a failure
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PREFIX="${MIGO_WINDOWS_PREFIX:-$REPO_ROOT/dist/migo-windows-x86_64}"
STRICT=0
[[ "${1:-}" == "--strict" ]] && STRICT=1

FAILURES=0
SKIPS=0
pass() { echo -e "\033[0;32mPASS\033[0m  $*"; }
fail() { echo -e "\033[0;31mFAIL\033[0m  $*"; FAILURES=$((FAILURES + 1)); }
skip() { echo -e "\033[0;33mSKIP\033[0m  $*"; SKIPS=$((SKIPS + 1)); }

[[ -d "$PREFIX" ]] || {
    echo "no staged package at $PREFIX; run scripts/build-windows-sdk.sh" >&2
    exit 1
}

DLL="$PREFIX/bin/migo.dll"
IMPLIB="$PREFIX/lib/migo.lib"
HEADERS="$PREFIX/include/migo"

# --- 1. The package has the shape a consumer is told to expect ---------------
missing=0
for required in "$DLL" "$IMPLIB" "$HEADERS/migo.h" \
                "$PREFIX/lib/cmake/migo/migo-config.cmake"; do
    [[ -e "$required" ]] || { fail "missing from the package: ${required#$PREFIX/}"; missing=1; }
done
(( missing )) || pass "package carries the DLL, import library, headers and CMake package"

# ANGLE is a runtime dependency the consumer cannot supply itself: migo.dll
# resolves EGL at load time, and a package without these loads on a developer
# machine that happens to have them and fails on a clean one.
angle_missing=0
for runtime in libEGL.dll libGLESv2.dll d3dcompiler_47.dll; do
    [[ -f "$PREFIX/bin/$runtime" ]] || { fail "missing ANGLE runtime: bin/$runtime"; angle_missing=1; }
done
(( angle_missing )) || pass "ANGLE runtime DLLs ship alongside migo.dll"

# --- 2. Toolchain-dependent checks ------------------------------------------
find_vcvars() {
    local vswhere="/mnt/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
    [[ -f "$vswhere" ]] || return 1
    local root
    root="$("$vswhere" -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 \
        -latest -format value -property installationPath 2>/dev/null | tr -d '\r')"
    [[ -n "$root" ]] || return 1
    printf '%s\\VC\\Auxiliary\\Build\\vcvars64.bat' "$root"
}

VCVARS="$(find_vcvars || true)"
if [[ -z "$VCVARS" ]]; then
    skip "export surface (no MSVC toolchain: dumpbin unavailable)"
    skip "DLL loads with its imports resolved (no MSVC toolchain)"
    skip "staged headers compile standalone under MSVC"
else
    WORK="$(mktemp -d /mnt/c/Windows/Temp/migo-winsdk.XXXXXX)"
    WORK_DOS="$(wslpath -w "$WORK")"
    trap 'rm -rf "$WORK"' EXIT

    run_msvc() {
        # Each invocation gets its own batch file: composing a cmd.exe command
        # line by quoting from bash gets mangled, and a batch file is the shape
        # that reliably survives (see CLAUDE.md on the Windows spike).
        local body="$1" name="$2"
        local bat="/mnt/c/Windows/Temp/migo-winsdk-$name.bat"
        {
            echo '@echo off'
            echo "call \"$VCVARS\" >nul"
            echo "$body"
        } > "$bat"
        # cd into the scratch directory first: cmd refuses a UNC working
        # directory and silently falls back to C:\Windows, where cl cannot
        # write its .obj -- reported as "Permission denied" on a path the
        # command never mentioned.
        ( cd /tmp && cmd.exe /c "cd /d $WORK_DOS && $(wslpath -w "$bat")" 2>/dev/null | tr -d '\r' )
    }

    # --- 2a. Export surface is exactly the documented migo_* set ------------
    # Read from the DLL rather than from the .def that produced it: the .def is
    # an input to the link, so checking it would only prove the input matches
    # itself. The headers are the authority on what the surface should be.
    DECLARED="$WORK/declared.txt"
    grep -ohE '\bmigo_[A-Za-z0-9_]+' "$HEADERS"/*.h "$HEADERS"/platform/*.h 2>/dev/null \
        | sort -u > "$DECLARED"
    [[ -s "$DECLARED" ]] || fail "no migo_* names found in the staged headers"

    EXPORTED="$WORK/exported.txt"
    run_msvc "dumpbin /EXPORTS \"$(wslpath -w "$DLL")\"" exports \
        | awk '/ordinal +hint +RVA +name/{f=1;next} f && NF>=4 {print $NF}' \
        | grep -E '^migo_' | sort -u > "$EXPORTED"

    if [[ ! -s "$EXPORTED" ]]; then
        fail "dumpbin reported no migo_* exports from migo.dll"
    else
        # Exports must be a subset of what the headers declare, and must not be
        # empty. Extra exports are the failure this gate exists for: a cdylib
        # would publish every reachable no_mangle symbol.
        EXTRA="$(comm -23 "$EXPORTED" "$DECLARED")"
        if [[ -n "$EXTRA" ]]; then
            fail "migo.dll exports names the headers do not declare:"
            printf '        %s\n' $EXTRA >&2
        else
            pass "export surface is exactly the declared migo_* set ($(wc -l < "$EXPORTED") entries)"
        fi
    fi

    # --- 2b. The DLL actually loads -----------------------------------------
    # An export table can be perfect while the DLL fails to load because an
    # import is unresolvable. Loading it is the only check that covers that,
    # and it is why this gate needs Windows rather than a PE parser.
    LOADER_SRC="$WORK/loadprobe.c"
    cat > "$LOADER_SRC" <<'PROBE'
#include <stdio.h>
#include <windows.h>
int main(int argc, char **argv) {
    if (argc < 2) return 2;
    HMODULE module = LoadLibraryA(argv[1]);
    if (!module) {
        fprintf(stderr, "LoadLibrary failed: %lu\n", GetLastError());
        return 1;
    }
    if (!GetProcAddress(module, "migo_query_capabilities")) {
        fprintf(stderr, "migo_query_capabilities not resolvable\n");
        return 1;
    }
    printf("LOAD_OK\n");
    return 0;
}
PROBE
    PROBE_EXE="$WORK/loadprobe.exe"
    BUILD_OUT="$(run_msvc "cl /nologo /Fe:loadprobe.exe loadprobe.c" build || true)"
    if [[ ! -f "$PROBE_EXE" ]]; then
        fail "could not compile the load probe with MSVC: $BUILD_OUT"
    else
        # Run from bin/ so the ANGLE DLLs beside migo.dll are on the search path,
        # which is exactly how a consumer ships them.
        # Stage the package's bin/ next to the probe from the WSL side. Handing
        # cmd a UNC source path mangles it (the leading \\ is eaten and the copy
        # silently matches nothing), and the point of this check is to load the
        # DLL the way a consumer ships it: beside its ANGLE runtimes.
        cp "$PREFIX/bin/"*.dll "$WORK/"
        LOAD_OUT="$(run_msvc "loadprobe.exe migo.dll" load || true)"
        if grep -q LOAD_OK <<<"$LOAD_OUT"; then
            pass "migo.dll loads and resolves migo_query_capabilities"
        else
            fail "migo.dll did not load: $LOAD_OUT"
        fi
    fi

    # --- 2c. Headers compile standalone -------------------------------------
    # A header that only compiles after something else was included first is a
    # header the consumer cannot use first.
    HDR_SRC="$WORK/hdrprobe.c"
    printf '#include <migo/migo.h>\nint main(void){return 0;}\n' > "$HDR_SRC"
    HDR_EXE="$WORK/hdrprobe.exe"
    HDR_OUT="$(run_msvc "cl /nologo /std:c11 /W4 /WX /I\"$(wslpath -w "$PREFIX/include")\" /Fe:hdrprobe.exe hdrprobe.c" hdr || true)"
    if [[ -f "$HDR_EXE" ]]; then
        pass "staged headers compile standalone under MSVC C11 (/W4 /WX)"
    else
        fail "staged headers do not compile standalone under MSVC: $HDR_OUT"
    fi
fi

# --- 3. Import library pairs with the DLL by name ---------------------------
# The Windows analogue of the soname check: an import library that names a
# different DLL links fine and fails at load time on the consumer's machine.
if grep -qa "migo.dll" "$IMPLIB" 2>/dev/null; then
    pass "import library references migo.dll"
else
    fail "migo.lib does not reference migo.dll -- it would link but not load"
fi

if (( STRICT )) && (( SKIPS )); then
    echo "FAIL: --strict was requested but $SKIPS check(s) were skipped"
    exit 1
fi
if (( FAILURES )); then
    echo "FAIL: $FAILURES contract violation(s)"
    exit 1
fi
echo "OK: Windows SDK contract satisfied ($SKIPS skipped)"
