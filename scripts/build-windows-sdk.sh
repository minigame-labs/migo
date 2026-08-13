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
# Default to the pinned, hash-verified location fetch-windows-angle.sh
# populates (contracts/artifact-manifest/windows-angle.lock.json), not an
# unpinned local directory: ANGLE ships no official binaries, so this is the
# only copy anything downstream can say something concrete about.
ANGLE_DIR_UNIX="${MIGO_WIN_ANGLE_DIR_UNIX:-$REPO_ROOT/engine/third_party/angle-windows}"

# ---- Preconditions -------------------------------------------------------
for f in rusty_v8.dll rusty_v8.dll.lib src_binding.rs; do
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
# The import library, so V8 is absorbed as a DLL dependency rather than as
# objects in this link. See the note at the top of this file: linking the static
# archive puts V8's own libc++ into the same link as Skia's MSVC STL, and they
# define std::terminate incompatibly.
V8_ARCHIVE_DOS="$V8_DIR_DOS\\rusty_v8.dll.lib"
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
set RUSTY_V8_ARCHIVE=${V8_ARCHIVE_DOS}
set RUSTY_V8_SRC_BINDING_PATH=${V8_DIR_DOS}\\src_binding.rs
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
  "${V8_ARCHIVE_DOS}" ^
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

# ---- Stage the package (WSL side) ----------------------------------------
info "staging package at $PREFIX"
rm -rf "$PREFIX"
mkdir -p "$PREFIX/include" "$PREFIX/lib" "$PREFIX/bin" "$PREFIX/lib/cmake/migo" "$PREFIX/share/migo"
cp -r "$REPO_ROOT/include/migo" "$PREFIX/include/"
cp "$OUT_UNIX/migo.lib" "$PREFIX/lib/migo.lib"
cp "$OUT_UNIX/migo.dll" "$PREFIX/bin/migo.dll"
# Runtime DLLs the process loads by name: V8 and ANGLE ship alongside migo.dll.
#
# libEGL and libGLESv2 are required, not best-effort. The previous form was
# `[[ -f ... ]] && cp` for all three, which silently shipped a package with no EGL
# at all when the ANGLE directory was incomplete -- and a missing EGL is not
# discoverable until a consumer's process fails to create a surface on a machine
# we cannot see. d3dcompiler_47.dll genuinely is optional: whether ANGLE needs it
# depends on how that ANGLE was built, so it is recorded in the package manifest
# when shipped rather than assumed either way.
cp "$V8_DIR_UNIX/rusty_v8.dll" "$PREFIX/bin/rusty_v8.dll"
for d in libEGL.dll libGLESv2.dll d3dcompiler_47.dll; do
    [[ -f "$ANGLE_DIR_UNIX/$d" ]] || {
        echo "[windows-sdk] required ANGLE runtime missing: $ANGLE_DIR_UNIX/$d" >&2
        echo "[windows-sdk] test-windows-sdk-contract.sh already requires all three, and this" >&2
        echo "[windows-sdk] script runs it with --strict below -- so copying leniently only" >&2
        echo "[windows-sdk] moved the failure later while leaving a package with no EGL on" >&2
        echo "[windows-sdk] disk in between. Point MIGO_WIN_ANGLE_DIR_UNIX at a complete" >&2
        echo "[windows-sdk] ANGLE build." >&2
        exit 1
    }
    cp "$ANGLE_DIR_UNIX/$d" "$PREFIX/bin/$d"
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
# The constraint that put a literal here is real and is kept: 0.1.0 was published
# before the C ABI had a Windows platform layer -- the DLL loaded, exported
# everything, and reported it could attach no surface kind -- and a released
# version names a fixed set of bytes, so those bytes must never ship under a
# version a consumer may already hold. A single forward-moving source satisfies
# that better than a detached literal did: this stopped tracking the rest of the
# repository entirely, so the Windows SDK announced a version no other platform
# had heard of.
MIGO_SDK_VERSION="$VERSION"
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

# ---- package manifest ----------------------------------------------------
# The one thing every other platform's package had and this one did not, which is
# why package-sdk.sh refuses a Windows prefix outright (see its error text) and why
# the published migo-windows-x86_64.tar.gz was a `tar` typed by hand rather than the
# reproducible path. The attestation names a single index file; this is it.
#
# The runtime_dependencies list is the part with teeth. A Windows consumer must
# redistribute these DLLs, and the process loads them by name -- so the package has
# to say which ones it shipped, and the contract has to check that bin/ contains
# exactly that set. Declaring it is what turns "the ANGLE directory was incomplete"
# from a silent shipping defect into a failure here.
info "writing the package manifest"
sha_of() { sha256sum "$1" | cut -d' ' -f1; }

# The same four names test-windows-sdk-contract.sh requires, kept in one order so
# the manifest and the gate cannot disagree about what a consumer must ship.
ALL_RUNTIME=(migo.dll rusty_v8.dll libEGL.dll libGLESv2.dll d3dcompiler_47.dll)

RUNTIME_JSON="$(
    first=1
    printf '['
    for d in "${ALL_RUNTIME[@]}"; do
        (( first )) || printf ','
        first=0
        printf '\n      {"file": "bin/%s", "sha256": "%s"}' "$d" "$(sha_of "$PREFIX/bin/$d")"
    done
    printf '\n    ]'
)"

cat > "$PREFIX/share/migo/windows-x86_64-manifest.json" <<MANIFEST
{
  "schema": "migo-windows-package-manifest/v1",
  "version": "$VERSION",
  "product_profile": "full",
  "build_type": "release",
  "target": "x86_64-pc-windows-msvc",
  "os": "windows",
  "abi": "msvc",
  "arch": "x86_64",
  "snapshot_policy": "none",
  "abi_note": "migo.dll is linked from the capi staticlib with an explicit .def export allowlist, so the export table is exactly the documented migo_* surface. V8 links against its own libc++ and therefore ships as a separate rusty_v8.dll rather than being absorbed into migo.dll.",
  "runtime_dependencies": $RUNTIME_JSON,
  "artifacts": {
    "lib/migo.lib": "$(sha_of "$PREFIX/lib/migo.lib")",
    "bin/migo.dll": "$(sha_of "$PREFIX/bin/migo.dll")"
  },
  "known_gaps": [
    "v8 startup snapshot: not embedded for this platform yet (runtime-v8/build.rs embeds for android and linux, not windows -- no SNAPSHOT-full-windows-x86_64.bin has been generated), so a cold start parses extension JS from source instead of deserialising a V8 heap",
    "this package is built with scripts/build-windows-sdk.sh by hand on a Windows-capable machine, not by CI: build-windows-sdk.sh uses wslpath/cmd.exe interop a GitHub windows-latest runner (no WSL) cannot run. The prebuilt V8 archive and ANGLE runtime it links ARE published (scripts/fetch-v8-archives.sh x86_64-pc-windows-msvc, scripts/fetch-windows-angle.sh), so a Windows-native CI job is possible; writing one is separate work.",
    "NuGet packaging is not implemented; the package is a CMake find_package tree"
  ]
}
MANIFEST

python3 -m json.tool "$PREFIX/share/migo/windows-x86_64-manifest.json" >/dev/null \
    || { echo "[windows-sdk] the generated manifest is not valid JSON" >&2; exit 1; }

info "staged:"
find "$PREFIX" -type f | sed "s|$PREFIX|  <prefix>|" | sort

# No CI runner builds this package, so this gate is the only thing standing
# between a broken link and a published SDK. Running it here rather than leaving
# it to the operator makes producing a package that fails its own contract
# impossible: windows-sdk-0.1.0 shipped a library that loaded, exported every
# entry point, and could not attach a window, because the gate was a separate
# step that answered a narrower question than "is this usable".
info "verifying the staged package against the Windows SDK contract"
MIGO_WINDOWS_PREFIX="$PREFIX" bash "$SCRIPT_DIR/test-windows-sdk-contract.sh" --strict

info "NOTE: NuGet packaging is the remaining packaging step; this stages a"
info "      linkable, runnable migo.dll + headers + CMake package."
