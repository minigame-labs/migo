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
#   * V8 on Windows is built with its own libc++ (clang-cl crashes on 32 torque
#     translation units without it), so V8 and Skia disagree about the C++
#     runtime. This links V8's IMPORT library and ships rusty_v8.dll beside
#     migo.dll, which is the only arrangement that keeps that libc++ out of this
#     link -- inside a DLL it does not participate in symbol resolution. Linking
#     the 201 MB static archive instead fails with LNK2005 on std::terminate,
#     defined strongly by libc++'s exception.obj and as a COMDAT by Skia's
#     MSVC-STL objects.
#     An earlier version of this script linked the archive and succeeded, which
#     is why the comment here used to claim the archive was equivalent. It only
#     linked because the C ABI had no Windows platform layer: nothing reached
#     GraphicsPlatform, so /OPT:REF discarded Skia's core and the two runtimes
#     never met. That artifact could not attach a surface at all.
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
# V8 inputs (rusty_v8.lib, rusty_v8.dll, rusty_v8.dll.lib, src_binding.rs) are
# read from the WSL-side engine/third_party/rusty_v8/x86_64-pc-windows-msvc --
# NOT the synced worktree copy, which is git-ignored (.gitignore) and so can
# never carry them (sync-worktree.sh only clones committed refs; pointing
# there once produced an empty path and a V8 build script panic
# ("系统找不到指定的路径") several layers away from the cause) -- verified
# against component-manifest.json and materialised by v8_materialise_windows
# before the DOS path handed to the link is ever computed. There is
# deliberately no override to point this at an arbitrary unverified location:
# scripts/fetch-v8-archives.sh x86_64-pc-windows-msvc is how you change what
# gets linked.
#
# Required env (same identities the spike uses):
#   MIGO_WIN_ANGLE_DIR_UNIX  WSL path to the ANGLE runtime DLLs (libEGL.dll,
#                     libGLESv2.dll, d3dcompiler_47.dll). Defaults to
#                     engine/third_party/angle-windows.
#   MIGO_WIN_PROXY    optional http proxy for the V8 crate's build script.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
source "$REPO_ROOT/platforms/windows/spike/lib.sh"
# shellcheck source=scripts/lib/windows-sdk-package.sh
source "$SCRIPT_DIR/lib/windows-sdk-package.sh"
# shellcheck source=scripts/lib/v8-materialise.sh
source "$SCRIPT_DIR/lib/v8-materialise.sh"

PREFIX="$REPO_ROOT/dist/migo-windows-x86_64"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --prefix) PREFIX="$2"; shift 2 ;;
        *) echo "[win-sdk] unknown argument: $1" >&2; exit 2 ;;
    esac
done

TRIPLE="$WIN_TARGET_TRIPLE"                      # x86_64-pc-windows-msvc
# WSL path is only for reading bytes to stage/check; the Windows link must use
# the synced worktree copy on a local disk -- a wslpath UNC path is unusable for
# the toolchain, which is the whole reason the worktree lives on C:.
V8_DIR_UNIX="$REPO_ROOT/engine/third_party/rusty_v8/x86_64-pc-windows-msvc"
# Default to the pinned, hash-verified location fetch-windows-angle.sh
# populates (contracts/artifact-manifest/windows-angle.lock.json), not an
# unpinned local directory: ANGLE ships no official binaries, so this is the
# only copy anything downstream can say something concrete about.
ANGLE_DIR_UNIX="${MIGO_WIN_ANGLE_DIR_UNIX:-$REPO_ROOT/engine/third_party/angle-windows}"

# ---- Preconditions -------------------------------------------------------
for f in rusty_v8.lib rusty_v8.dll rusty_v8.dll.lib src_binding.rs; do
    [[ -f "$V8_DIR_UNIX/$f" ]] || { echo "[win-sdk] missing Windows V8 artifact: $V8_DIR_UNIX/$f" >&2; exit 1; }
done
for f in libEGL.dll libGLESv2.dll d3dcompiler_47.dll; do
    [[ -f "$ANGLE_DIR_UNIX/$f" ]] || {
        echo "[win-sdk] missing ANGLE runtime DLL: $ANGLE_DIR_UNIX/$f" >&2
        echo "[win-sdk] fetch the pinned set with: bash scripts/fetch-windows-angle.sh" >&2
        exit 1
    }
done
require_synced_worktree

# The repository's single release-version source. Read rather than derived from a
# crate manifest, and with no fallback: a default here ships a package labelled
# with a version nobody chose, which is how the Linux and HarmonyOS SDKs could
# have been built as `0.1.0` from a tree that was not.
# shellcheck source=scripts/lib/release-version.sh
source "$SCRIPT_DIR/lib/release-version.sh"

VERSION="$(read_release_version "$REPO_ROOT")"

# ---- Generate the export allowlist (.def) from the headers ----------------
# Same source of truth the Linux migo.map uses -- the documented migo_* names.
DEF_UNIX="$WIN_TMP_UNIX/migo.def"
mkdir -p "$WIN_TMP_UNIX"
windows_sdk_generate_def "$REPO_ROOT/include/migo" "$DEF_UNIX"
DEF_DOS="$(wslpath -w "$DEF_UNIX")"

# ---- Build the staticlib, capture native libs, and link the DLL on Windows ----
# The import library, so V8 is absorbed as a DLL dependency rather than as
# objects in this link. See the note at the top of this file: linking the static
# archive puts V8's own libc++ into the same link as Skia's MSVC STL, and they
# define std::terminate incompatibly.
#
# Verifies rusty_v8.lib + src_binding.rs against component-manifest.json and
# materialises all four V8 inputs (including the unhashed DLL/import-lib pair
# -- see v8_materialise_windows's own comment) under a content-addressed path,
# rather than linking whatever sits at $V8_DIR_UNIX unverified.
#
# Materialised under $WIN_TMP_UNIX (a WSL-visible path into the Windows-local
# scratch dir every other build output here already uses), not under
# $REPO_ROOT/engine/target the way build-linux-sdk.sh does: link.exe refuses a
# \\wsl.localhost\... UNC path outright (LNK1104, "cannot open file"), so the
# materialised archive has to live on the C: drive link.exe actually reads
# from -- the same reason DEF_UNIX and OUT_UNIX below are WIN_TMP_UNIX paths,
# not plain repo-relative ones.
v8_materialise_windows "$V8_DIR_UNIX" "$WIN_TMP_UNIX/v8-materialised" \
    || { echo "[win-sdk] failed to materialise the Windows V8 archive" >&2; exit 1; }
V8_MATERIALISED_DIR_UNIX="$(dirname "$V8_MATERIALISED_ARCHIVE")"
V8_MATERIALISED_ARCHIVE_DOS="$(wslpath -w "$V8_MATERIALISED_ARCHIVE")"
V8_MATERIALISED_BINDING_DOS="$(wslpath -w "$V8_MATERIALISED_BINDING")"
OUT_DOS="$WIN_TMP_UNIX/sdk-out"
OUT_UNIX="$WIN_TMP_UNIX/sdk-out"
mkdir -p "$OUT_UNIX"
OUT_DOS="$(wslpath -w "$OUT_UNIX")"
VCVARS_DOS="$(find_vcvars64)"
PROXY_LINES=""
[[ -n "${MIGO_WIN_PROXY:-}" ]] && PROXY_LINES="set HTTPS_PROXY=${MIGO_WIN_PROXY}
set HTTP_PROXY=${MIGO_WIN_PROXY}"

WIN_TARGET_UNIX="$(win_to_unix_path "$WIN_TARGET_DOS")"
WIN_CARGO_HOME_UNIX="$(win_to_unix_path "$WIN_CARGO_HOME_DOS")"

# The manual link needs the non-system lib search dirs cargo/rustc knows via
# cargo:rustc-link-search but link.exe does not: skia-bindings and
# windows-targets stage their .libs in the build output / registry, not on the
# MSVC LIB path (which vcvars sets for the system libs). Discovered dynamically
# -- the skia-bindings hash and windows-targets versions are build-specific and
# must never be hardcoded.
#
# CALLED AFTER THE BUILD, NEVER BEFORE. This used to run at script top level,
# which made the whole flow depend on a warm CARGO_TARGET_DIR: the paths it
# scans for are *produced* by the build, so on a cold target it found nothing,
# LIB got no skia directory, and the link died with a bare
# `LNK1181: cannot open input file 'skparagraph.lib'` -- measured, not
# theorised. `--print native-static-libs` emits bare library NAMES with no
# directory component, so LIB is the only channel these directories have.
# Returns non-zero when skia's directory is not there. That is the fail-closed
# half: handing back a partial list lets link.exe run and fail with a bare
# `LNK1181: cannot open input file 'skparagraph.lib'`, which reads like a
# corrupt build rather than a search path that was never assembled. An absence
# assertion needs to be distinguishable from an empty scan.
discover_link_search_dirs() {
    local _extra=() _skia _d
    _skia="$(find "$WIN_TARGET_UNIX/$TRIPLE/release/build" -path '*skia-bindings-*/out/skia/skparagraph.lib' 2>/dev/null | head -1)"
    [[ -n "$_skia" ]] || return 1
    _extra+=("$(wslpath -w "$(dirname "$_skia")")")
    [[ -d "$WIN_TARGET_UNIX/$TRIPLE/release/gn_out/obj" ]] && _extra+=("$(wslpath -w "$WIN_TARGET_UNIX/$TRIPLE/release/gn_out/obj")")
    while IFS= read -r _d; do _extra+=("$(wslpath -w "$_d")"); done \
        < <(find "$WIN_CARGO_HOME_UNIX/registry" -path '*windows_x86_64_msvc-*/lib' -type d 2>/dev/null)
    (IFS=';'; printf '%s' "${_extra[*]}")
}

# Every environment fact both Windows stages need. Emitted once and reused: two
# hand-kept copies of a developer environment drift, and the failures that
# produces (a stale INCLUDE, an NDK clang ahead of LLVM) surface far from here.
emit_win_preamble() {
    cat <<PRE
@echo off
rem An inherited variable of this name makes %errorlevel% expand to it forever.
set "ERRORLEVEL="
call "${VCVARS_DOS}" >nul
rem Whitelist PATH: an Android NDK on PATH shadows clang-cl and breaks bindgen,
rem so this is an allowlist, not the inherited PATH. LLVM stays first so
rem skia-bindings still resolves clang-cl to it; the MSVC tools bin is added
rem (after LLVM, so clang-cl still wins) because this script calls link.exe
rem directly for the DLL link, and cargo building only the staticlib never
rem invoked the linker so vcvars' own PATH addition was not missed until now.
set "PATH=${MIGO_WIN_LLVM_DIR_DOS}\\bin;%VCToolsInstallDir%bin\\Hostx64\\x64;%USERPROFILE%\\.cargo\\bin;${WIN_TOOLS_DOS};%SystemRoot%\\system32;%SystemRoot%;%SystemRoot%\\System32\\Wbem;${WIN_EXTRA_PATH_DOS}"
set CARGO_HOME=${WIN_CARGO_HOME_DOS}
set CARGO_TARGET_DIR=${WIN_TARGET_DOS}
set INCLUDE=${WIN_HEADERS_DOS};%INCLUDE%
set RUSTY_V8_ARCHIVE=${V8_MATERIALISED_ARCHIVE_DOS}
set RUSTY_V8_SRC_BINDING_PATH=${V8_MATERIALISED_BINDING_DOS}
${PROXY_LINES}
PRE
}

# ---- Stage 1: compile the staticlib and record what it needs linked ---------
# Split from the link deliberately. The link's search path is derived from files
# this stage produces, so the two cannot share one batch: whatever a single batch
# computed up front would describe the previous build, and on a cold target would
# describe nothing at all.
BATCH_BUILD="$WIN_TMP_UNIX/build-sdk-compile.bat"
{ emit_win_preamble; cat <<BAT
cd /d ${WIN_WORKTREE_DOS}\\engine || exit /b 90

echo [win-sdk] building capi staticlib (release)
cargo build -p migo-capi --release --target ${TRIPLE}
set STEP=%errorlevel%
if not "%STEP%"=="0" exit /b %STEP%

echo [win-sdk] capturing native-static-libs
rem CARGO_TERM_COLOR=never: this note is parsed, not read by a person. Left
rem unset, a forced-color environment wraps the note in ANSI codes whose
rem trailing reset glues onto the last -l token and corrupts the parsed link
rem line. See build-android-sdk.sh for the reproduction that found this.
set CARGO_TERM_COLOR=never
cargo rustc -p migo-capi --lib --release --target ${TRIPLE} --crate-type staticlib -- --print native-static-libs > "${OUT_DOS}\\native-libs.txt" 2>&1
rem cargo prints the note to stderr; the line we need starts with "native-static-libs:".

set STATICLIB=${WIN_TARGET_DOS}\\${TRIPLE}\\release\\migo_capi.lib
if not exist "%STATICLIB%" ( echo [win-sdk] staticlib not found: %STATICLIB% & exit /b 91 )
echo === BUILD_EXIT=0 ===
exit /b 0
BAT
} > "$BATCH_BUILD"

RUN_LOG="$WIN_TMP_UNIX/build-sdk.log"
info "building on Windows (this compiles the full stack; first run is ~minutes)"
run_windows_batch "$BATCH_BUILD" | tee "$RUN_LOG"
BUILD_RC="${PIPESTATUS[0]}"
[[ "$BUILD_RC" -eq 0 ]] || { echo "[win-sdk] staticlib build failed (exit $BUILD_RC) -- see $RUN_LOG" >&2; exit 1; }

# ---- Now the search path exists, so discover it -----------------------------
if ! EXTRA_LIB_DOS="$(discover_link_search_dirs)"; then
    echo "[win-sdk] the staticlib built but skia-bindings' output directory was not found under" >&2
    echo "[win-sdk]   $WIN_TARGET_UNIX/$TRIPLE/release/build/*skia-bindings-*/out/skia/" >&2
    echo "[win-sdk] link.exe resolves the bare library names cargo reports through LIB alone, so" >&2
    echo "[win-sdk] linking without that directory fails with LNK1181 rather than anything clearer." >&2
    exit 1
fi
info "link search dirs discovered after the build: $(awk -F';' '{print NF}' <<<"$EXTRA_LIB_DOS")"

# ---- Stage 2: link the DLL --------------------------------------------------
BATCH_LINK="$WIN_TMP_UNIX/build-sdk-link.bat"
{ emit_win_preamble; cat <<BAT
rem link.exe searches LIB for input .libs; vcvars sets it for the system libs.
rem Prepend the skia-bindings / windows-targets / V8 dirs the manual link needs.
set "LIB=${EXTRA_LIB_DOS};%LIB%"
cd /d ${WIN_WORKTREE_DOS}\\engine || exit /b 90

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
  "${V8_MATERIALISED_ARCHIVE_DOS}" ^
  %NATIVE_LIBS%
set STEP=%errorlevel%
echo === LINK_EXIT=%STEP% ===
exit /b %STEP%
BAT
} > "$BATCH_LINK"

info "linking on Windows"
run_windows_batch "$BATCH_LINK" | tee -a "$RUN_LOG"
LINK_RC="${PIPESTATUS[0]}"
[[ "$LINK_RC" -eq 0 ]] || { echo "[win-sdk] link failed (exit $LINK_RC) -- see $RUN_LOG" >&2; exit 1; }

[[ -f "$OUT_UNIX/migo.dll" ]] || { echo "[win-sdk] link produced no migo.dll -- see $RUN_LOG" >&2; exit 1; }
[[ -f "$OUT_UNIX/migo.lib" ]] || { echo "[win-sdk] link produced no import lib migo.lib" >&2; exit 1; }
info "linked: migo.dll ($(stat -c %s "$OUT_UNIX/migo.dll") bytes) + import lib migo.lib"

# ---- Stage the package, CMake package, and manifest (WSL side) -----------
# Shared with scripts/build-windows-sdk-native.sh: see scripts/lib/windows-sdk-
# package.sh for what lives there and why -- the export .def, the staged files,
# the CMake find_package(migo) tree, and windows-x86_64-manifest.json are all
# identical regardless of which script produced the linked migo.dll.
windows_sdk_stage_package "$PREFIX" "$REPO_ROOT/include/migo" \
    "$OUT_UNIX/migo.lib" "$OUT_UNIX/migo.dll" "$V8_MATERIALISED_DIR_UNIX/rusty_v8.dll" "$ANGLE_DIR_UNIX"
windows_sdk_write_cmake_package "$PREFIX" "$VERSION"
info "writing the package manifest"
windows_sdk_write_manifest "$PREFIX" "$VERSION"

# This local WSL path still has no other gate before this point, so this call is
# the only thing standing between a broken link and a published SDK on this
# path. Running it here rather than leaving it to the operator makes producing a
# package that fails its own contract impossible: windows-sdk-0.1.0 shipped a
# library that loaded, exported every entry point, and could not attach a
# window, because the gate was a separate step that answered a narrower
# question than "is this usable". (scripts/build-windows-sdk-native.sh, used by
# CI, runs this same contract as its own explicit CI step instead of
# self-invoking it, for per-step visibility in the Actions log.)
info "verifying the staged package against the Windows SDK contract"
MIGO_WINDOWS_PREFIX="$PREFIX" bash "$SCRIPT_DIR/test-windows-sdk-contract.sh" --strict

info "NOTE: NuGet packaging is the remaining packaging step; this stages a"
info "      linkable, runnable migo.dll + headers + CMake package."
