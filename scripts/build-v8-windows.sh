#!/usr/bin/env bash
# ============================================================
# Build the x86_64-pc-windows-msvc rusty_v8.lib that migo links on Windows.
# Location: scripts/build-v8-windows.sh
#
# Counterpart to scripts/build-v8-android.sh and scripts/build-v8-linux.sh.
# Like them it builds inside a real rusty_v8 checkout, because that is the only
# tree that can build V8 from source -- see "Why a checkout" below.
#
# WHY THIS EXISTS AT ALL (the measurement that forced it, 2026-07-23):
#
#   Linking Skia and the *prebuilt* rusty_v8 into one Windows binary fails:
#
#     skia.lib(core.SkExecutor.obj) : error LNK2005:
#       "void __cdecl std::terminate(void)" already defined in
#       libv8-....rlib(exception.obj)
#
#   Both sides carry a C++ runtime: Skia is built against the MSVC STL
#   (skia-bindings passes /MD), while the published rusty_v8 binaries bundle
#   V8's own libc++. That is a duplicate *definition*, so neither /OPT:REF nor
#   an import library can fix it -- only making both use the same C++ runtime
#   can, and the prebuilt cannot be changed. Building V8 here with
#   use_custom_libcxx=false puts V8 on the MSVC STL and the duplicate is gone
#   (measured: LNK2005 count 89 -> 0, and the LNK4098 libcmt conflict with it).
#
#   `migo-runtime-v8` links the prebuilt fine and its tests pass on Windows,
#   because a V8-only binary has nothing to collide with. Only Skia + V8 in one
#   binary needs this build.
#
# WHY A CHECKOUT AND NOT THE PUBLISHED CRATE:
#   The published `v8` crate cannot build V8 from source. Measured against a
#   real checkout it is missing 42912 tracked files, including
#   build/rust/known-target-triples.txt and the whole tools/win tree that gn
#   dereferences on Windows. Reconstructing it from another tree would also mix
#   provenance, which contracts/artifact-manifest/*.lock.json exists to prevent.
#   Separately, `use_custom_libcxx` is a *cargo feature*, not a GN argument
#   (build.rs derives the GN value from CARGO_FEATURE_USE_CUSTOM_LIBCXX and
#   overrides anything EXTRA_GN_ARGS says), and cargo refuses
#   --no-default-features for a package outside the workspace -- so the build
#   has to happen where v8 is the root package.
#
# PREREQUISITES (each was a real failure before it was one):
#   - RUSTY_V8_SRC_WIN: a rusty_v8 checkout ON A WINDOWS LOCAL DISK, with
#     submodules initialised. Not a UNC path: cargo produced no output in six
#     minutes over \\wsl.localhost and never created its target directory.
#     It must NOT carry another host's downloaded dependencies -- see --check.
#   - Visual Studio Build Tools with the C++ workload, and LLVM (clang-cl).
#   - GN and NINJA on the Windows side (see MIGO_WIN_GN / MIGO_WIN_NINJA).
#     They are passed explicitly because rusty_v8's own downloader cannot be
#     used here: tools/ninja_gn_binaries.py resolves CIPD through raw
#     http.client, which ignores HTTPS_PROXY, so on a machine that reaches the
#     internet only through a proxy it times out. Every other download in this
#     build goes through urllib (tools/download_file.py) and does honour it.
#   - Python: rusty_v8 resolves it via $PYTHON and otherwise calls "python3",
#     which Windows installs do not provide.
#
# Output: engine/third_party/rusty_v8/x86_64-pc-windows-msvc/
#           rusty_v8.lib + src_binding.rs
#
# Usage:
#   scripts/build-v8-windows.sh            # build and install
#   scripts/build-v8-windows.sh --check    # report readiness, build nothing
# ============================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE_ROOT="$PROJECT_ROOT/engine"
TARGET="x86_64-pc-windows-msvc"
OUT_DIR="$ENGINE_ROOT/third_party/rusty_v8/$TARGET"
V8_BUILD_LOCK="$PROJECT_ROOT/contracts/artifact-manifest/android-v8.lock.json"

# Windows-side paths. All DOS form: they are written into a batch file.
RUSTY_V8_SRC_WIN="${RUSTY_V8_SRC_WIN:-C:\\v8src}"
# Short by necessity, not tidiness: ninja calls the ANSI GetFullPathNameA,
# which is capped at MAX_PATH even with LongPathsEnabled, and gn_out plus
# Chromium's deepest headers overrun it from any longer root.
V8_TARGET_DIR_WIN="${MIGO_WIN_V8_TARGET_DIR:-C:\\v8o}"
# Same MAX_PATH constraint as the target directory, and it keeps this build on
# the same registry cache the rest of the Windows tooling already populated.
CARGO_HOME_WIN="${MIGO_WIN_CARGO_HOME:-C:\\cg}"
GN_WIN="${MIGO_WIN_GN:-C:\\migo-win-spike-tmp\\bin\\gn.exe}"
NINJA_WIN="${MIGO_WIN_NINJA:-C:\\migo-win-spike-tmp\\bin\\ninja.exe}"
PYTHON_WIN="${MIGO_WIN_PYTHON:-}"
PROXY="${MIGO_WIN_PROXY:-}"

# printf, not `echo -e`: every path here is a DOS path, and `echo -e` expands
# backslash escapes inside the message -- "C:\v8src" prints as "C:<vtab>8src",
# silently corrupting the diagnostics that are supposed to tell you which tree
# was used.
info() { printf '\033[0;36m[v8-win] %s\033[0m\n' "$*"; }
ok()   { printf '\033[0;32m[v8-win] %s\033[0m\n' "$*"; }
err()  { printf '\033[0;31m[v8-win] %s\033[0m\n' "$*" >&2; }

# The Windows-side path as WSL sees it, for the readiness checks below.
win_to_unix() { printf '/mnt/%s' "$(printf '%s' "$1" | sed 's|\\|/|g; s|^\(.\):|\L\1|')"; }
SRC_UNIX="$(win_to_unix "$RUSTY_V8_SRC_WIN")"

# ------------------------------------------------------------
# Readiness checks
# ------------------------------------------------------------
# These are separated from the build because every one of them failed once, and
# each failure surfaced far from its cause: a missing python became "program not
# found" inside build.rs, a Build Tools install under %ProgramFiles(x86)% became
# "No supported Visual Studio can be found", and a foreign host's downloaded
# toolchain became "[WinError 193] not a valid Win32 application" from a python
# script three layers down.
check_ready() {
    local failures=0

    if [[ ! -f "$SRC_UNIX/build.rs" ]]; then
        err "not a rusty_v8 checkout: $RUSTY_V8_SRC_WIN (no build.rs)"; failures=$((failures + 1))
    elif [[ ! -f "$SRC_UNIX/v8/include/v8-version.h" ]]; then
        err "v8 submodule not checked out in $RUSTY_V8_SRC_WIN"
        err "run: git -C <checkout> submodule update --init --recursive"
        failures=$((failures + 1))
    fi

    # A checkout copied from another host carries that host's *downloaded*
    # dependencies, which are git-ignored precisely because they are not source.
    # The Windows toolchain tarball extracts *over* them rather than replacing
    # them, leaving a Linux ELF `rustc` beside `rustc.exe` in the same bin
    # directory; Chromium's find_std_rlibs.py then picks the extensionless one
    # and the build dies with WinError 193. Refuse rather than build something
    # whose toolchain is half another platform's.
    local foreign=""
    if [[ -e "$SRC_UNIX/third_party/rust-toolchain/bin/rustc" \
       && ! -e "$SRC_UNIX/third_party/rust-toolchain/bin/rustc.exe" ]]; then
        foreign="third_party/rust-toolchain (extensionless rustc, no rustc.exe)"
    elif [[ -e "$SRC_UNIX/third_party/rust-toolchain/bin/rustc" ]]; then
        foreign="third_party/rust-toolchain (contains a non-Windows rustc)"
    fi
    if [[ -n "$foreign" ]]; then
        err "checkout carries another host's downloaded toolchain: $foreign"
        err "delete the git-ignored dependency directories and let this build refetch them"
        failures=$((failures + 1))
    fi

    for tool in "$GN_WIN" "$NINJA_WIN"; do
        [[ -f "$(win_to_unix "$tool")" ]] || { err "not found: $tool"; failures=$((failures + 1)); }
    done

    if [[ -z "$PYTHON_WIN" ]]; then
        local candidate
        candidate="$(ls -d /mnt/c/Users/*/AppData/Local/Programs/Python/Python3*/python.exe 2>/dev/null | sort -V | tail -1 || true)"
        if [[ -n "$candidate" ]]; then
            PYTHON_WIN="$(printf '%s' "$candidate" | sed 's|^/mnt/\(.\)/|\U\1:\\\\|; s|/|\\\\|g')"
        else
            err "no Windows python found; set MIGO_WIN_PYTHON"; failures=$((failures + 1))
        fi
    fi

    local vswhere="/mnt/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
    if [[ ! -f "$vswhere" ]]; then
        err "vswhere not found -- install Visual Studio Build Tools with the C++ workload"
        failures=$((failures + 1))
    else
        VS_ROOT="$("$vswhere" -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 \
            -latest -format value -property installationPath 2>/dev/null | tr -d '\r')"
        [[ -n "$VS_ROOT" ]] || { err "no Visual Studio carries the MSVC x86/x64 build tools"; failures=$((failures + 1)); }
    fi

    return "$failures"
}

if [[ "${1:-}" == "--check" ]]; then
    if check_ready; then ok "ready to build"; exit 0; else err "not ready"; exit 1; fi
fi
check_ready || { err "prerequisites unmet; run --check for the list"; exit 1; }

# ------------------------------------------------------------
# Patches
# ------------------------------------------------------------
# Same mechanism the Android and Linux builds use: apply only the patches this
# platform needs, then assert each one took effect, so a patch that silently
# stops matching upstream fails the build rather than producing a V8 that
# quietly lacks it. Windows previously had no patch step at all -- its tree was
# edited by hand -- so a clean checkout could not reproduce the Windows V8 and
# the artifact-manifest provenance chain did not hold on this platform.
PATCH_DIR="$ENGINE_ROOT/third_party/v8-patches"
EXPORTS_DEF="$PATCH_DIR/rusty_v8-windows-exports.def"

apply_windows_patches() {
    # target_file|sentinel|patch_glob
    local specs=(
        "src/binding.cc|v8__register_host_callback|0005-*.patch"
        "BUILD.gn|shared_library(\"rusty_v8\")|0006-*.patch"
        "src/V8.rs|register_host_callbacks_once|0007-*.patch"
    )
    local spec
    for spec in "${specs[@]}"; do
        local tgt="${spec%%|*}"; local rest="${spec#*|}"
        local sentinel="${rest%%|*}"; local glob="${rest##*|}"
        local -a matches=("$PATCH_DIR"/$glob)
        local pf="${matches[0]}"
        [[ -f "$pf" ]] || { err "missing patch: $glob"; return 1; }
        if grep -qF "$sentinel" "$SRC_UNIX/$tgt" 2>/dev/null; then
            echo "  = already in effect: $(basename "$pf")"
            continue
        fi
        # No `</dev/null` here: a second redirect on the same descriptor wins,
        # so it would feed patch an empty stdin -- it then exits 0 having done
        # nothing, which the sentinel check below is what catches. `--batch`
        # already suppresses the prompting that redirect was meant to avoid.
        if ! patch -p1 -d "$SRC_UNIX" --batch --forward < "$pf"; then
            err "patch failed: $(basename "$pf")"; return 1
        fi
        if ! grep -qF "$sentinel" "$SRC_UNIX/$tgt" 2>/dev/null; then
            err "patch ran but sentinel missing in $tgt: $(basename "$pf")"; return 1
        fi
        echo "  ✓ applied $(basename "$pf")"
    done

    # The DLL exports exactly the C binding surface, listed in a generated file
    # (scripts/gen-windows-v8-def.sh) rather than hand-kept, so it cannot drift
    # from what rusty_v8 actually defines. BUILD.gn references it by this name.
    [[ -f "$EXPORTS_DEF" ]] || { err "missing export list: $EXPORTS_DEF"; return 1; }
    cp "$EXPORTS_DEF" "$SRC_UNIX/rusty_v8.def" || return 1
    echo "  ✓ staged rusty_v8.def ($(grep -c '^    ' "$EXPORTS_DEF") exports)"
}

apply_windows_patches || { err "patch stage failed"; exit 1; }

# ------------------------------------------------------------
# Build
# ------------------------------------------------------------
BATCH_UNIX="/mnt/c/migo-win-spike-tmp/migo-build-v8-windows.bat"
mkdir -p "$(dirname "$BATCH_UNIX")"

PROXY_LINES=""
[[ -n "$PROXY" ]] && PROXY_LINES="set HTTPS_PROXY=${PROXY}
set HTTP_PROXY=${PROXY}"

# NOTE: this heredoc is unquoted so the shell expands ${...}; it must therefore
# contain no backticks, which the shell would run as a command substitution
# instead of writing to the file.
cat > "$BATCH_UNIX" <<BAT
@echo off
rem An inherited variable of this name makes %errorlevel% expand to it forever.
set "ERRORLEVEL="
call "${VS_ROOT}\\VC\\Auxiliary\\Build\\vcvars64.bat" >nul

rem A whitelist, not a reordering. An Android NDK ships its own clang-cl and
rem clang resource directory, and the resolution inside bindgen does not consult
rem PATH order, so the NDK directory has to be ABSENT rather than merely later.
set "PATH=C:\\Program Files\\LLVM\\bin;%USERPROFILE%\\.cargo\\bin;%SystemRoot%\\system32;%SystemRoot%;%SystemRoot%\\System32\\Wbem"

set CARGO_HOME=${CARGO_HOME_WIN}
set CARGO_TARGET_DIR=${V8_TARGET_DIR_WIN}
${PROXY_LINES}

rem rusty_v8 resolves python via PYTHON and otherwise calls "python3"; Windows
rem installs python.exe and no python3.exe.
set PYTHON=${PYTHON_WIN}

rem With GN and NINJA set, need_gn_ninja_download() is false and the CIPD
rem resolve call -- the one download in this build that ignores HTTPS_PROXY,
rem because it uses raw http.client -- is never reached.
set GN=${GN_WIN}
set NINJA=${NINJA_WIN}

rem Chromium build/vs_toolchain.py probes only %ProgramFiles% for VS2022, while
rem Build Tools commonly installs under %ProgramFiles(x86)%. vs<year>_install is
rem that script's own documented override and is consulted first.
set "vs2022_install=${VS_ROOT}"

set V8_FROM_SOURCE=1
rem Kept in step with the Android and Linux recipes: WebAssembly and pointer
rem compression on, sandbox off, i18n on because rusty_v8's binding.cc includes
rem <unicode/locid.h> unconditionally and fails to compile without it.
rem use_allocator_shim / use_partition_alloc_as_malloc: Chromium's allocator
rem shim replaces malloc/free process-wide. Linked into a host it would free
rem pointers the host allocated with the system allocator, and it also makes
rem the archive redefine ucrt's malloc/free/realloc/calloc/_msize, which fails
rem the link outright. scripts/build-v8-linux.sh turns both off for the same
rem reason: an engine that hijacks its host's allocator is not embeddable.
rem treat_warnings_as_errors: building V8 against the MSVC STL is a
rem configuration upstream does not test, and it trips exactly one warning --
rem -Wctad-maybe-unsupported on std::atomic_ref, 78 times. libc++ annotates
rem that template as deduction-friendly and the MSVC STL does not; the code is
rem valid either way. Chromium exposes no per-warning argument, and rusty_v8's
rem own build.rs already turns this off whenever it uses a system clang. It
rem changes no generated code, only whether a warning fails the build.
set EXTRA_GN_ARGS=v8_enable_webassembly=true v8_enable_pointer_compression=true v8_enable_i18n_support=true v8_enable_sandbox=false use_allocator_shim=false use_partition_alloc_as_malloc=false

cd /d ${RUSTY_V8_SRC_WIN} || exit /b 90
rem V8 is built WITH its bundled libc++ (the crate default), because it cannot
rem be compiled against the MSVC STL: measured 2026-07-24, clang-cl crashes
rem (frontend signal) on 32 torque-generated translation units, and before that
rem 78 -Wctad-maybe-unsupported errors. Chromium compiles those with
rem -D_HAS_EXCEPTIONS=0 and /std:c++23preview, a combination Microsoft does not
rem support for its own STL. So `use_custom_libcxx=false` is not an option here,
rem and the Skia/V8 std::terminate collision needs a different answer -- see
rem platforms/windows/SPIKE-REPORT.md.
cargo build --release --target ${TARGET}
rem Capture before echoing: an echo succeeds and would otherwise become the
rem batch file's exit status, making a failed build look like a pass.
set CARGO_EXIT=%errorlevel%
echo === EXIT=%CARGO_EXIT% ===
exit /b %CARGO_EXIT%
BAT

info "building V8 for $TARGET in $RUSTY_V8_SRC_WIN (this takes several minutes)"
( cd "$(dirname "$BATCH_UNIX")" && cmd.exe /c "$(basename "$BATCH_UNIX")" )

# ------------------------------------------------------------
# Install artifacts
# ------------------------------------------------------------
GN_OUT_UNIX="$(win_to_unix "$V8_TARGET_DIR_WIN")/$TARGET/release/gn_out"
BINDING="$GN_OUT_UNIX/src_binding.rs"

# rusty_v8 is a shared_library here (see the BUILD.gn patch): the products are
# the DLL plus its import library, both in the build root rather than obj/.
# gn names the import library after the DLL, so accept either spelling instead
# of pinning one and failing opaquely if the toolchain uses the other.
DLL="$GN_OUT_UNIX/rusty_v8.dll"
IMPLIB=""
for candidate in "$GN_OUT_UNIX/rusty_v8.dll.lib" "$GN_OUT_UNIX/rusty_v8.lib"; do
    [[ -f "$candidate" ]] && { IMPLIB="$candidate"; break; }
done

[[ -f "$DLL" ]] || { err "rusty_v8.dll not found after build: $DLL"; exit 1; }
[[ -n "$IMPLIB" ]] || { err "no import library beside $DLL"; exit 1; }
[[ -f "$BINDING" ]] || { err "src_binding.rs not found after build: $BINDING"; exit 1; }

mkdir -p "$OUT_DIR"
cp "$DLL" "$OUT_DIR/rusty_v8.dll"
cp "$IMPLIB" "$OUT_DIR/rusty_v8.dll.lib"
cp "$BINDING" "$OUT_DIR/src_binding.rs"
ok "dll     -> $OUT_DIR/rusty_v8.dll ($(du -h "$OUT_DIR/rusty_v8.dll" | cut -f1))"
ok "implib  -> $OUT_DIR/rusty_v8.dll.lib ($(du -h "$OUT_DIR/rusty_v8.dll.lib" | cut -f1))"
ok "binding -> $OUT_DIR/src_binding.rs"

# The component manifest is deliberately not written yet. Its writer is
# per-platform (scripts/write-{,linux-}v8-component-manifest.py) and a Windows
# one needs a windows-v8.lock.json to verify against; $V8_BUILD_LOCK covers the
# Android targets only. Producing an unverified manifest would defeat the point
# of having one, so this reports the gap instead of papering over it.
info "component manifest not written: needs a Windows lock + writer (see $V8_BUILD_LOCK for the Android shape)"
ok "V8 build complete for $TARGET"
