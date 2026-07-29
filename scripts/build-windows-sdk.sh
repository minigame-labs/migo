#!/usr/bin/env bash
# Build the Windows x64 SDK: migo.dll (+ import lib) linked from the capi
# staticlib with an explicit export surface, staged with headers, the V8 and
# ANGLE runtime DLLs, and a CMake package a non-cargo MSVC consumer can use.
#
# This mirrors scripts/build-linux-sdk.sh. The shape is the same on purpose:
# capi is a *staticlib*, and the shipped library is linked from it with an
# export allowlist rather than produced by a Rust `cdylib`, because a cdylib
# makes rustc emit its own export table listing every reachable `#[no_mangle]`
# symbol (rusty_v8_*, skia wrappers, ...) and the public surface must be exactly
# the documented migo_* entry points -- nothing else.
#
# The Windows differences from the Linux script:
#   * The export allowlist is a `.def` file consumed by `link /DEF:`, not a GNU
#     version script. It is generated from include/migo/*.h so it cannot drift
#     from the headers.
#   * V8 on Windows is built as a shared library (so V8 and Skia share the MSVC
#     STL instead of colliding on std::terminate), which yields both a DLL and a
#     201 MB static archive in gn_out. This links the ARCHIVE, not the import
#     library: migo.dll absorbs V8 exactly the way it absorbs librusty_v8.a on
#     Linux, and `dumpbin /DEPENDENTS migo.dll` lists no rusty_v8.dll. The DLL
#     form matters for how V8 is *compiled* (its libc++ stays internal); it is
#     not what gets shipped. Verified 2026-07-29 against the built artifact --
#     an earlier version of this comment claimed the opposite.
#   * The MSVC linker's /OPT:REF is the analog of --gc-sections: skia-bindings
#     compiles one translation unit with JPEG/PDF/pathops wrappers that Skia is
#     built without, so it references symbols that do not exist; /OPT:REF must
#     discard the unreferenced wrappers.
#
# Source stays in WSL; the compile+link runs on a Windows local-disk worktree,
# for the same reason the spike does it (UNC is unusable for cargo). Staging of
# the produced binaries into the package tree is done here on the WSL side.
#
# Usage: scripts/build-windows-sdk.sh [--prefix WSL_DIR]
#
# Required env (same identities the spike uses):
#   MIGO_WIN_V8_DIR   DOS path to the Windows V8 artifacts (rusty_v8.dll,
#                     rusty_v8.lib, src_binding.rs). Defaults to the WSL-side
#                     engine/third_party/rusty_v8/x86_64-pc-windows-msvc, which
#                     is where build-v8-windows.sh puts them.
#
#                     NOT the synced worktree copy: that whole directory is
#                     git-ignored (.gitignore), so sync-worktree.sh -- which
#                     clones -- can never carry it across. Pointing there
#                     produced an empty path and a V8 build script panic
#                     ("系统找不到指定的路径") several layers away from the cause.
#   MIGO_WIN_ANGLE_DIR DOS path to the ANGLE runtime DLLs (libEGL.dll,
#                     libGLESv2.dll, d3dcompiler_47.dll). Defaults to the spike tmp.
#   MIGO_WIN_PROXY    optional http proxy for the V8 crate's build script.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
source "$REPO_ROOT/platforms/windows/spike/lib.sh"

PREFIX="$REPO_ROOT/dist/migo-windows-x86_64"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --prefix) PREFIX="$2"; shift 2 ;;
        *) echo "[win-sdk] unknown argument: $1" >&2; exit 2 ;;
    esac
done
info() { echo -e "\033[0;36m[win-sdk] $*\033[0m"; }

TRIPLE="$WIN_TARGET_TRIPLE"                      # x86_64-pc-windows-msvc
# WSL path is only for reading bytes to stage/check; the Windows link must use
# the synced worktree copy on a local disk -- a wslpath UNC path is unusable for
# the toolchain, which is the whole reason the worktree lives on C:.
V8_DIR_UNIX="$REPO_ROOT/engine/third_party/rusty_v8/x86_64-pc-windows-msvc"
V8_DIR_DOS="${MIGO_WIN_V8_DIR:-$(wslpath -w "$V8_DIR_UNIX")}"
ANGLE_DIR_UNIX="${MIGO_WIN_ANGLE_DIR_UNIX:-$WIN_TMP_UNIX/angle}"
[[ -d "$ANGLE_DIR_UNIX" ]] || ANGLE_DIR_UNIX="$WIN_TMP_UNIX"   # spike staged them flat once

# ---- Preconditions -------------------------------------------------------
for f in rusty_v8.dll rusty_v8.lib src_binding.rs; do
    [[ -f "$V8_DIR_UNIX/$f" ]] || { echo "[win-sdk] missing Windows V8 artifact: $V8_DIR_UNIX/$f" >&2; exit 1; }
done
for f in libEGL.dll libGLESv2.dll; do
    [[ -f "$ANGLE_DIR_UNIX/$f" ]] || { echo "[win-sdk] missing ANGLE runtime DLL: $ANGLE_DIR_UNIX/$f" >&2; exit 1; }
done
require_synced_worktree

VERSION="$(grep -m1 '^version' "$REPO_ROOT/engine/crates/capi/Cargo.toml" | cut -d'"' -f2)"

# ---- Generate the export allowlist (.def) from the headers ----------------
# Same source of truth the Linux migo.map uses -- the documented migo_* names.
DEF_UNIX="$WIN_TMP_UNIX/migo.def"
mkdir -p "$WIN_TMP_UNIX"
{
    echo "EXPORTS"
    grep -ohE '\bmigo_[a-z0-9_]+[[:space:]]*\(' "$REPO_ROOT"/include/migo/*.h \
        | tr -d '( \t' | sort -u | sed 's/^/    /'
} > "$DEF_UNIX"
DEF_COUNT=$(( $(wc -l < "$DEF_UNIX") - 1 ))
info "generated export allowlist: $DEF_COUNT migo_* symbols"
[[ "$DEF_COUNT" -ge 20 ]] || { echo "[win-sdk] suspiciously few exports ($DEF_COUNT); headers not found?" >&2; exit 1; }
DEF_DOS="$(wslpath -w "$DEF_UNIX")"

# ---- Build the staticlib, capture native libs, and link the DLL on Windows ----
V8_ARCHIVE_DOS="$V8_DIR_DOS\\rusty_v8.lib"
OUT_DOS="$WIN_TMP_UNIX/sdk-out"
OUT_UNIX="$WIN_TMP_UNIX/sdk-out"
mkdir -p "$OUT_UNIX"
OUT_DOS="$(wslpath -w "$OUT_UNIX")"
VCVARS_DOS="$(find_vcvars64)"
PROXY_LINES=""
[[ -n "${MIGO_WIN_PROXY:-}" ]] && PROXY_LINES="set HTTPS_PROXY=${MIGO_WIN_PROXY}
set HTTP_PROXY=${MIGO_WIN_PROXY}"

# The manual link needs the non-system lib search dirs cargo/rustc knows via
# cargo:rustc-link-search but link.exe does not: skia-bindings and
# windows-targets stage their .libs in the build output / registry, not on the
# MSVC LIB path (which vcvars sets for the system libs). Discovered dynamically
# -- the skia-bindings hash and windows-targets versions are build-specific and
# must never be hardcoded. (These exist because the staticlib was already built;
# a from-clean run builds it in the batch below, and the dirs are stable across
# the incremental relink this discovers before.)
WIN_TARGET_UNIX="$(win_to_unix_path "$WIN_TARGET_DOS")"
WIN_CARGO_HOME_UNIX="$(win_to_unix_path "$WIN_CARGO_HOME_DOS")"
_extra=()
_skia="$(find "$WIN_TARGET_UNIX/$TRIPLE/release/build" -path '*skia-bindings-*/out/skia/skparagraph.lib' 2>/dev/null | head -1)"
[[ -n "$_skia" ]] && _extra+=("$(wslpath -w "$(dirname "$_skia")")")
[[ -d "$WIN_TARGET_UNIX/$TRIPLE/release/gn_out/obj" ]] && _extra+=("$(wslpath -w "$WIN_TARGET_UNIX/$TRIPLE/release/gn_out/obj")")
while IFS= read -r _d; do _extra+=("$(wslpath -w "$_d")"); done \
    < <(find "$WIN_CARGO_HOME_UNIX/registry" -path '*windows_x86_64_msvc-*/lib' -type d 2>/dev/null)
EXTRA_LIB_DOS="$(IFS=';'; printf '%s' "${_extra[*]}")"
info "extra link search dirs: ${#_extra[@]}"

BATCH="$WIN_TMP_UNIX/build-sdk.bat"
cat > "$BATCH" <<BAT
@echo off
set "ERRORLEVEL="
call "${VCVARS_DOS}" >nul
rem Whitelist PATH: an Android NDK on PATH shadows clang-cl and breaks bindgen,
rem so this is an allowlist, not the inherited PATH. LLVM stays first so
rem skia-bindings still resolves clang-cl to it; the MSVC tools bin is added
rem (after LLVM, so clang-cl still wins) because THIS script calls link.exe
rem directly for the DLL link, and cargo building only the staticlib never
rem invoked the linker so vcvars' own PATH addition was not missed until now.
set "PATH=${MIGO_WIN_LLVM_DIR_DOS}\\bin;%VCToolsInstallDir%bin\\Hostx64\\x64;%USERPROFILE%\\.cargo\\bin;${WIN_TOOLS_DOS};%SystemRoot%\\system32;%SystemRoot%;%SystemRoot%\\System32\\Wbem;${WIN_EXTRA_PATH_DOS}"
rem link.exe searches LIB for input .libs; vcvars sets it for the system libs.
rem Prepend the skia-bindings / windows-targets / V8 dirs the manual link needs.
set "LIB=${EXTRA_LIB_DOS};%LIB%"
set CARGO_HOME=${WIN_CARGO_HOME_DOS}
set CARGO_TARGET_DIR=${WIN_TARGET_DOS}
set INCLUDE=${WIN_HEADERS_DOS};%INCLUDE%
set RUSTY_V8_ARCHIVE=${V8_ARCHIVE_DOS}
set RUSTY_V8_SRC_BINDING_PATH=${V8_DIR_DOS}\\src_binding.rs
${PROXY_LINES}
cd /d ${WIN_WORKTREE_DOS}\\engine || exit /b 90

echo [win-sdk] building capi staticlib (release)
cargo build -p migo-capi --release --target ${TRIPLE}
set STEP=%errorlevel%
if not "%STEP%"=="0" exit /b %STEP%

echo [win-sdk] capturing native-static-libs
cargo rustc -p migo-capi --lib --release --target ${TRIPLE} --crate-type staticlib -- --print native-static-libs > "${OUT_DOS}\\native-libs.txt" 2>&1
rem cargo prints the note to stderr; the line we need starts with "native-static-libs:".

set STATICLIB=${WIN_TARGET_DOS}\\${TRIPLE}\\release\\migo_capi.lib
if not exist "%STATICLIB%" ( echo [win-sdk] staticlib not found: %STATICLIB% & exit /b 91 )

echo [win-sdk] reading native libs
rem The line is "  note: native-static-libs: <libs>", so match anywhere (not /b)
rem and strip everything up to and including the marker with cmd's *string= form.
for /f "delims=" %%a in ('findstr /c:"native-static-libs:" "${OUT_DOS}\\native-libs.txt"') do set "RAWLIBS=%%a"
if not defined RAWLIBS ( echo [win-sdk] no native-static-libs line found & exit /b 95 )
set "NATIVE_LIBS=%RAWLIBS:*native-static-libs: =%"

echo [win-sdk] linking migo.dll
link /NOLOGO /DLL /DEF:"${DEF_DOS}" ^
  /OUT:"${OUT_DOS}\\migo.dll" ^
  /IMPLIB:"${OUT_DOS}\\migo.lib" ^
  /OPT:REF /OPT:ICF ^
  "%STATICLIB%" ^
  "${V8_ARCHIVE_DOS}" ^
  %NATIVE_LIBS%
set STEP=%errorlevel%
echo === LINK_EXIT=%STEP% ===
exit /b %STEP%
BAT

RUN_LOG="$WIN_TMP_UNIX/build-sdk.log"
info "building on Windows (this compiles the full stack; first run is ~minutes)"
run_windows_batch "$BATCH" | tee "$RUN_LOG"

[[ -f "$OUT_UNIX/migo.dll" ]] || { echo "[win-sdk] link produced no migo.dll -- see $RUN_LOG" >&2; exit 1; }
[[ -f "$OUT_UNIX/migo.lib" ]] || { echo "[win-sdk] link produced no import lib migo.lib" >&2; exit 1; }
info "linked: migo.dll ($(stat -c %s "$OUT_UNIX/migo.dll") bytes) + import lib migo.lib"

# ---- Stage the package (WSL side) ----------------------------------------
info "staging package at $PREFIX"
rm -rf "$PREFIX"
mkdir -p "$PREFIX/include" "$PREFIX/lib" "$PREFIX/bin" "$PREFIX/lib/cmake/migo" "$PREFIX/share/migo"
cp -r "$REPO_ROOT/include/migo" "$PREFIX/include/"
cp "$OUT_UNIX/migo.lib" "$PREFIX/lib/migo.lib"
cp "$OUT_UNIX/migo.dll" "$PREFIX/bin/migo.dll"
# Runtime DLLs the process loads by name: V8 and ANGLE ship alongside migo.dll.
cp "$V8_DIR_UNIX/rusty_v8.dll" "$PREFIX/bin/rusty_v8.dll"
for d in libEGL.dll libGLESv2.dll d3dcompiler_47.dll; do
    [[ -f "$ANGLE_DIR_UNIX/$d" ]] && cp "$ANGLE_DIR_UNIX/$d" "$PREFIX/bin/$d"
done

# ---- CMake package -------------------------------------------------------
# A SHARED IMPORTED target: the consumer links the import lib (migo.lib) and the
# DLL ships in bin/. The target carries three requirements so a consumer needs
# only target_link_libraries(app migo::migo):
#   * MIGO_USE_SHARED  -> the header's MIGO_API becomes __declspec(dllimport),
#     which is the correct decoration for consuming a DLL.
#   * c_std_11         -> the headers assert layout with C11 _Static_assert, and
#     MSVC's default C dialect (C89) does not know it; the target opts the
#     consumer into C11 rather than making every consumer remember /std:c11.
#   * the include dir and both the .lib (IMPLIB) and .dll (LOCATION) locations.
MIGO_SDK_VERSION="0.1.0"
cat > "$PREFIX/lib/cmake/migo/migo-config.cmake" <<'CMAKE'
# Generated by scripts/build-windows-sdk.sh -- do not edit.
#
# Consume with the MSVC toolchain:
#   cmake -S <your_app> -B <build> -DCMAKE_PREFIX_PATH=<this package prefix>
#   find_package(migo REQUIRED)
#   target_link_libraries(app PRIVATE migo::migo)
# The DLLs in <prefix>/bin (migo.dll, rusty_v8.dll, ANGLE) must be on PATH at
# runtime, or copied next to the executable.
cmake_minimum_required(VERSION 3.16)

get_filename_component(MIGO_PREFIX "${CMAKE_CURRENT_LIST_DIR}/../../.." ABSOLUTE)

set(MIGO_VERSION "@MIGO_SDK_VERSION@")
set(MIGO_INCLUDE_DIRS "${MIGO_PREFIX}/include")
set(MIGO_IMPLIB "${MIGO_PREFIX}/lib/migo.lib")
set(MIGO_DLL "${MIGO_PREFIX}/bin/migo.dll")

add_library(migo::migo SHARED IMPORTED)
set_target_properties(migo::migo PROPERTIES
    IMPORTED_IMPLIB "${MIGO_IMPLIB}"
    IMPORTED_LOCATION "${MIGO_DLL}"
    INTERFACE_INCLUDE_DIRECTORIES "${MIGO_INCLUDE_DIRS}"
    INTERFACE_COMPILE_DEFINITIONS "MIGO_USE_SHARED"
    INTERFACE_COMPILE_FEATURES "c_std_11")

set(migo_FOUND TRUE)
CMAKE
sed -i "s|@MIGO_SDK_VERSION@|$MIGO_SDK_VERSION|" "$PREFIX/lib/cmake/migo/migo-config.cmake"

cat > "$PREFIX/lib/cmake/migo/migo-config-version.cmake" <<CMAKE
# Generated by scripts/build-windows-sdk.sh -- do not edit.
set(PACKAGE_VERSION "$MIGO_SDK_VERSION")
if(PACKAGE_VERSION VERSION_LESS PACKAGE_FIND_VERSION)
    set(PACKAGE_VERSION_COMPATIBLE FALSE)
else()
    set(PACKAGE_VERSION_COMPATIBLE TRUE)
    if(PACKAGE_VERSION VERSION_EQUAL PACKAGE_FIND_VERSION)
        set(PACKAGE_VERSION_EXACT TRUE)
    endif()
endif()
CMAKE

info "staged:"
find "$PREFIX" -type f | sed "s|$PREFIX|  <prefix>|" | sort
info "NOTE: NuGet packaging and the V8 component-manifest provenance are the next"
info "      steps; this stages a linkable, runnable migo.dll + headers + CMake package."
