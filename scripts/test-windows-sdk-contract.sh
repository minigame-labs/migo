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
# shellcheck source=scripts/lib/python-cmd.sh
source "$SCRIPT_DIR/lib/python-cmd.sh"
# shellcheck source=scripts/lib/windows-sdk-package.sh
source "$SCRIPT_DIR/lib/windows-sdk-package.sh"
# shellcheck source=scripts/lib/windows-native-toolchain.sh
source "$SCRIPT_DIR/lib/windows-native-toolchain.sh"
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

# --- 1b. The package states what it is ---------------------------------------
# Every other platform's build script writes a package manifest; this one did not,
# which is why scripts/package-sdk.sh refuses a Windows prefix and why the published
# Windows archive was a `tar` typed by hand instead of the reproducible path. The
# attestation binds the package bytes to that file, so its absence is not cosmetic.
# The structural checking lives in scripts/lib/windows_package_manifest.py.
MANIFEST="$PREFIX/share/migo/windows-x86_64-manifest.json"
if [[ ! -f "$MANIFEST" ]]; then
    fail "missing package manifest: share/migo/windows-x86_64-manifest.json"
elif manifest_report="$("$(python_cmd)" "$SCRIPT_DIR/lib/windows_package_manifest.py" "$MANIFEST" "$PREFIX")"; then
    pass "manifest declares its runtime DLLs and artifact hashes, and they match the package"
else
    while IFS= read -r line; do
        [[ -n "$line" ]] && fail "manifest: $line"
    done <<< "$manifest_report"
fi

# Runtime DLLs the consumer cannot supply itself: migo.dll resolves EGL through
# ANGLE and imports V8 from rusty_v8.dll, both by name at load time. A package
# missing one loads on a developer machine that happens to have it and fails on
# a clean one. The load probe below is what proves resolution actually works;
# this list exists so a missing file is named instead of surfacing as a bare
# LoadLibrary error code.
runtime_missing=0
for runtime in libEGL.dll libGLESv2.dll d3dcompiler_47.dll rusty_v8.dll; do
    [[ -f "$PREFIX/bin/$runtime" ]] || { fail "missing runtime DLL: bin/$runtime"; runtime_missing=1; }
done
(( runtime_missing )) || pass "ANGLE and V8 runtime DLLs ship alongside migo.dll"

# --- 2. Toolchain-dependent checks ------------------------------------------
# Two environments run this file: WSL (this project's dev machine, crossing
# into a synced Windows worktree via cmd.exe -- see build-windows-sdk.sh) and
# native Windows (CI's windows-latest runner, and any plain Windows box with
# Git for Windows + Visual Studio Build Tools, no WSL involved -- see
# build-windows-sdk-native.sh). `wslpath` exists only in the former, so its
# presence is the one signal this script needs to tell them apart.
IS_WSL=0
command -v wslpath >/dev/null 2>&1 && IS_WSL=1

to_dos() {
    if (( IS_WSL )); then wslpath -w "$1"; else cygpath -w "$1"; fi
}

if (( IS_WSL )); then
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
    TOOLCHAIN_READY=0
    [[ -n "$VCVARS" ]] && TOOLCHAIN_READY=1
else
    # Native: nothing to locate. The caller already loaded vcvars into this
    # process's own PATH/INCLUDE/LIB -- the same precondition
    # build-windows-sdk-native.sh enforces before it will link anything -- so
    # dumpbin/cl being absent here means the caller skipped that step, not
    # that this script should go probe for a Visual Studio install itself.
    TOOLCHAIN_READY=0
    { command -v dumpbin.exe >/dev/null 2>&1 && command -v cl.exe >/dev/null 2>&1; } && TOOLCHAIN_READY=1
    (( TOOLCHAIN_READY )) && windows_native_ensure_msvc_link_wins
fi

if (( ! TOOLCHAIN_READY )); then
    skip "export surface (no MSVC toolchain: dumpbin unavailable)"
    skip "DLL loads with its imports resolved (no MSVC toolchain)"
    skip "staged headers compile standalone under MSVC"
else
    if (( IS_WSL )); then
        WORK="$(mktemp -d /mnt/c/Windows/Temp/migo-winsdk.XXXXXX)"
    else
        WORK="$(mktemp -d)"
    fi
    WORK_DOS="$(to_dos "$WORK")"
    trap 'rm -rf "$WORK"' EXIT

    # Runs one MSVC tool invocation from inside $WORK and prints its output.
    #   run_msvc <name> -- <argv...>
    # WSL crosses into Windows via a generated batch file + cmd.exe: composing
    # a cmd.exe command line by quoting from bash gets mangled, and a batch
    # file is the shape that reliably survives. Any argument containing a
    # space (a "Program Files"-style DOS path) is quoted when the batch line
    # is assembled, since the array form no longer arrives pre-quoted the way
    # a hand-built string did.
    # Natively, dumpbin/cl/the probe exe this builds are already directly
    # callable in this process's own PATH -- vcvars is already loaded here, so
    # there is no boundary to cross, and going through cmd.exe would be
    # unnecessary indirection. `PATH=".:$PATH"` makes the just-built
    # loadprobe.exe runnable as a bare name from $WORK: unlike cmd.exe, bash
    # does not search the current directory by default. MSYS_NO_PATHCONV is
    # scoped to just this invocation, not exported for the script: every
    # dumpbin/cl flag here is single-slash (/EXPORTS, /nologo, /W4, ...),
    # which MSYS's default path-conversion heuristic can mistake for a POSIX
    # path and mangle -- but path *arguments* passed in (to_dos "$DLL", etc.)
    # are already pre-converted DOS strings, so nothing here depends on that
    # heuristic running either way.
    run_msvc() {
        local name="$1"; shift
        [[ "${1:-}" == "--" ]] && shift
        if (( IS_WSL )); then
            local bat="/mnt/c/Windows/Temp/migo-winsdk-$name.bat" line="" a
            for a in "$@"; do
                if [[ "$a" == *" "* ]]; then line+="\"$a\" "; else line+="$a "; fi
            done
            {
                echo '@echo off'
                echo "call \"$VCVARS\" >nul"
                echo "$line"
            } > "$bat"
            # cd into the scratch directory first: cmd refuses a UNC working
            # directory and silently falls back to C:\Windows, where cl cannot
            # write its .obj -- reported as "Permission denied" on a path the
            # command never mentioned.
            ( cd /tmp && cmd.exe /c "cd /d $WORK_DOS && $(wslpath -w "$bat")" 2>/dev/null | tr -d '\r' )
        else
            # 2>/dev/null to match the WSL branch above, which discards
            # whatever the wrapped cmd.exe session sent to its own stderr.
            ( cd "$WORK" && PATH=".:$PATH" MSYS_NO_PATHCONV=1 "$@" 2>/dev/null )
        fi
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
    run_msvc exports -- dumpbin /EXPORTS "$(to_dos "$DLL")" \
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
#include <stdint.h>
#include <stdio.h>
#include <string.h>
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
    /* Loading and resolving is not the same as being usable. A build whose C
     * ABI has no platform layer for this OS exports every entry point, loads
     * cleanly, and then reports it can attach nothing -- which is exactly what
     * shipped once. Ask the library what it supports, the way a host must
     * before it builds a window. */
    typedef struct { uint32_t struct_size; uint32_t abi_version;
                     uint32_t abi_version_min; uint32_t abi_version_max;
                     uint64_t platform_kinds; } Caps;
    typedef int (*QueryFn)(Caps *);
    QueryFn query = (QueryFn)(void *)GetProcAddress(module, "migo_query_capabilities");
    Caps caps;
    memset(&caps, 0, sizeof caps);
    caps.struct_size = (uint32_t)sizeof caps;
    caps.abi_version = 1;
    if (query(&caps) != 0) {
        fprintf(stderr, "migo_query_capabilities failed\n");
        return 1;
    }
    if (caps.platform_kinds == 0) {
        fprintf(stderr, "the library supports no surface platform (platform_kinds=0)\n");
        return 1;
    }
    /* MIGO_PLATFORM_WIN32_HWND is 2. */
    if ((caps.platform_kinds & (1ull << 2)) == 0) {
        fprintf(stderr, "cannot attach a Win32 HWND (platform_kinds=0x%llx)\n",
                (unsigned long long)caps.platform_kinds);
        return 1;
    }
    printf("LOAD_OK platform_kinds=0x%llx\n", (unsigned long long)caps.platform_kinds);
    return 0;
}
PROBE
    PROBE_EXE="$WORK/loadprobe.exe"
    BUILD_OUT="$(run_msvc build -- cl /nologo /Fe:loadprobe.exe loadprobe.c || true)"
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
        LOAD_OUT="$(run_msvc load -- loadprobe.exe migo.dll || true)"
        if grep -q LOAD_OK <<<"$LOAD_OUT"; then
            pass "migo.dll loads and reports it can attach a Win32 HWND"
        else
            fail "migo.dll is not usable as a Windows host runtime: $LOAD_OUT"
        fi
    fi

    # --- 2c. Headers compile standalone -------------------------------------
    # A header that only compiles after something else was included first is a
    # header the consumer cannot use first.
    HDR_SRC="$WORK/hdrprobe.c"
    printf '#include <migo/migo.h>\nint main(void){return 0;}\n' > "$HDR_SRC"
    HDR_EXE="$WORK/hdrprobe.exe"
    HDR_OUT="$(run_msvc hdr -- cl /nologo /std:c11 /W4 /WX "/I$(to_dos "$PREFIX/include")" /Fe:hdrprobe.exe hdrprobe.c || true)"
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
