#!/usr/bin/env bash
# ============================================================
# Build ANGLE (libEGL.dll + libGLESv2.dll) for Windows from source.
# Location: scripts/build-angle-windows.sh
#
# WHY THIS EXISTS AT ALL:
#   contracts/artifact-manifest/windows-angle.lock.json's own "provenance_gap"
#   note says it plainly: the libEGL.dll/libGLESv2.dll this project has
#   shipped for Windows x64 were staged by earlier exploratory work before
#   that lock file existed, and their exact acquisition was never recorded.
#   ANGLE (google/angle) itself publishes no official prebuilt Windows
#   binaries, so there has never been an upstream URL to pin against -- only
#   a self-built, fully-provenanced archive closes that gap, the same way
#   scripts/build-v8-windows.sh closed it for the V8 archive. This script is
#   that closing move, and the first ANGLE-from-source pipeline this project
#   has had for any Windows architecture -- arm64 has no prebuilt DLLs to
#   have inherited in the first place, so it needs this script to exist at
#   all before it can ship.
#
# PREREQUISITES (each was a real failure before it was one -- see the three
# patches in engine/third_party/angle-patches/ for the ones baked into the
# checkout itself; these are the ones that stay external):
#   - ANGLE_SRC_WIN: an ANGLE checkout ON A WINDOWS LOCAL DISK (not a UNC
#     path -- same MAX_PATH and cross-filesystem-tooling reasons as
#     build-v8-windows.sh's RUSTY_V8_SRC_WIN), already gclient sync/runhooks'd
#     with target_os=["win"] via NATIVE Windows Python (not WSL's -- running
#     gclient under WSL's own Linux python makes Chromium's build tooling
#     take a Linux-hosted cross-compile path that needs a `ciopfs` FUSE mount,
#     which cannot be created on WSL's 9p-backed /mnt/c at all).
#   - Visual Studio Build Tools with the C++ workload, ATL, and (for aarch64)
#     the ARM64 cross-tools component.
#   - DEPOT_TOOLS_WIN_TOOLCHAIN=0 in the environment (external-contributor
#     mode: use the local VS install above, not Google's internal one).
#
# WHAT THIS DELIBERATELY DOES NOT BUILD:
#   angle_enable_wgpu is forced off. That backend alone is what pulls in
#   Dawn's vendored DirectX Shader Compiler, which needs ATL headers this
#   project's Windows dev machines were not carrying (Microsoft.VisualStudio
#   .Component.VC.ATL is not part of the core VC.Tools.x86.x64 workload) --
#   and confirmed via `ninja -n` dry run with the backend off: `dxil`, the
#   other thing that backend needed and this SDK's newest version does not
#   ship in its versioned bin/ directory, no longer appears anywhere in the
#   2965-target build graph either. migo does not use WebGPU; the D3D11 and
#   native-Vulkan backends this build keeps are what it actually ships.
#
# Output: engine/third_party/angle-windows-<public-arch>/
#           libEGL.dll + libGLESv2.dll (+ import libs) + d3dcompiler_47.dll
#         Deliberately NOT engine/third_party/angle-windows/ (x64, unsuffixed):
#         that directory is fetch-windows-angle.sh's existing, working,
#         already-consumed-by-build-windows-sdk.sh location for the pinned
#         x64 DLLs. Landing a differently-provenanced x64 build there by
#         default would silently replace a proven artifact with an unproven
#         one; this script's x64 output stays opt-in and side-by-side until
#         it has been through the same scrutiny before superseding the pin.
#
# Usage:
#   scripts/build-angle-windows.sh [aarch64|x86_64]            # build and install
#   scripts/build-angle-windows.sh [aarch64|x86_64] --check    # report readiness, build nothing
# ============================================================
set -euo pipefail

ARCH="x86_64"
CHECK_ONLY=0
for arg in "$@"; do
    case "$arg" in
        aarch64|x86_64) ARCH="$arg" ;;
        --check) CHECK_ONLY=1 ;;
        *) echo "usage: $0 [aarch64|x86_64] [--check]" >&2; exit 2 ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE_ROOT="$PROJECT_ROOT/engine"
case "$ARCH" in
    aarch64) GN_CPU="arm64"; PUBLIC_ARCH="arm64"; VS_ARM64_COMPONENT="Microsoft.VisualStudio.Component.VC.Tools.ARM64" ;;
    x86_64)  GN_CPU="x64";   PUBLIC_ARCH="x64";   VS_ARM64_COMPONENT="" ;;
esac
OUT_DIR="$ENGINE_ROOT/third_party/angle-windows-$PUBLIC_ARCH"
PATCH_DIR="$ENGINE_ROOT/third_party/angle-patches"

# Windows-side paths. All DOS form: they are written into a batch file.
ANGLE_SRC_WIN="${ANGLE_SRC_WIN:-C:\\anglesrc}"
GN_WIN="${MIGO_WIN_GN:-}"
NINJA_WIN="${MIGO_WIN_NINJA:-C:\\migo-win-spike-tmp\\bin\\ninja.exe}"
PYTHON_WIN="${MIGO_WIN_PYTHON:-}"
PROXY="${MIGO_WIN_PROXY:-}"
NUM_JOBS_WIN="${MIGO_WIN_NUM_JOBS:-}"

info() { printf '\033[0;36m[angle-win] %s\033[0m\n' "$*"; }
ok()   { printf '\033[0;32m[angle-win] %s\033[0m\n' "$*"; }
err()  { printf '\033[0;31m[angle-win] %s\033[0m\n' "$*" >&2; }

win_to_unix() { printf '/mnt/%s' "$(printf '%s' "$1" | sed 's|\\|/|g; s|^\(.\):|\L\1|')"; }
# bash's own dirname expects forward slashes; every path here is DOS-form.
win_dirname() { printf '%s' "${1%\\*}"; }
find_windows_python() {
    local finder="/mnt/c/Windows/System32/where.exe" candidate
    [[ -f "$finder" ]] || return 1
    candidate="$("$finder" python.exe 2>/dev/null | tr -d '\r' | head -n 1)"
    [[ -n "$candidate" ]] || return 1
    printf '%s' "$candidate"
}
SRC_UNIX="$(win_to_unix "$ANGLE_SRC_WIN")"

# ------------------------------------------------------------
# Readiness checks
# ------------------------------------------------------------
check_ready() {
    local failures=0

    if [[ ! -d "$SRC_UNIX/build" || ! -f "$SRC_UNIX/BUILD.gn" ]]; then
        err "not an ANGLE checkout: $ANGLE_SRC_WIN (no BUILD.gn / build/)"; failures=$((failures + 1))
    fi

    if [[ -z "$PYTHON_WIN" ]]; then
        local candidate
        if candidate="$(find_windows_python)"; then
            PYTHON_WIN="$candidate"
        else
            err "no Windows python found; set MIGO_WIN_PYTHON"; failures=$((failures + 1))
        fi
    fi

    local vswhere="/mnt/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
    if [[ ! -f "$vswhere" ]]; then
        err "vswhere not found -- install Visual Studio Build Tools with the C++ workload"
        failures=$((failures + 1))
    else
        # ATL is required (Dawn's own MSSupport code needs atlbase.h even with
        # angle_enable_wgpu=false -- the target still exists in the build
        # graph, just unreferenced by libEGL/libGLESv2's own deps -- no,
        # correction: confirmed via the dry run in the header comment that
        # with wgpu off, ATL is not needed either. Left unrequired here
        # deliberately: requiring it would block a build that does not need
        # it, and if a future change re-enables wgpu, check_ready failing to
        # mention ATL is exactly the kind of gap this project's own history
        # says surfaces far from its cause -- so if that happens, it will
        # reproduce the exact `atlbase.h` error this recipe's header
        # documents, not a silent miscompile.
        local -a requires=(-requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64)
        [[ -n "$VS_ARM64_COMPONENT" ]] && requires+=(-requires "$VS_ARM64_COMPONENT")
        VS_ROOT="$("$vswhere" -products '*' "${requires[@]}" \
            -latest -format value -property installationPath 2>/dev/null | tr -d '\r')"
        if [[ -z "$VS_ROOT" ]]; then
            err "no Visual Studio carries the MSVC build tools this target needs ($ARCH)"
            failures=$((failures + 1))
        fi
    fi

    local sdk_include="/mnt/c/Program Files (x86)/Windows Kits/10/Include"
    SDK_VERSION="$(ls "$sdk_include" 2>/dev/null | sort -V | tail -1)"
    [[ -n "$SDK_VERSION" ]] || { err "cannot determine the Windows SDK version"; failures=$((failures + 1)); }

    local d3dcompiler_src="/mnt/c/Program Files (x86)/Windows Kits/10/Redist/D3D/$GN_CPU/d3dcompiler_47.dll"
    [[ -f "$d3dcompiler_src" ]] || { err "d3dcompiler_47.dll not found: $d3dcompiler_src"; failures=$((failures + 1)); }

    return "$failures"
}

if [[ "$CHECK_ONLY" == "1" ]]; then
    if check_ready; then ok "ready to build $ARCH"; exit 0; else err "not ready"; exit 1; fi
fi
check_ready || { err "prerequisites unmet; run --check for the list"; exit 1; }

# ------------------------------------------------------------
# Patches
# ------------------------------------------------------------
# Same mechanism build-v8-windows.sh uses (shared library, not
# ANGLE-specific despite the "v8_" prefix in its function names): apply only
# the patches this platform needs, then assert each one took effect. All
# three of ANGLE's own external-contributor-path bugs this project has found
# live inside the `build` git submodule, which is why `patch -p1`
# (git-submodule-boundary-agnostic) rather than `git apply` is what applies
# them -- see that library's own header for why.
# shellcheck source=scripts/lib/v8-patch-apply.sh
source "$SCRIPT_DIR/lib/v8-patch-apply.sh"

apply_angle_patches() {
    local glob
    for glob in "0001-*.patch" "0002-*.patch" "0003-*.patch"; do
        v8_require_patch "$SRC_UNIX/build" "$PATCH_DIR" "$glob" || return 1
    done
}
apply_angle_patches || { err "patch stage failed"; exit 1; }

# ------------------------------------------------------------
# Toolchain fetch (idempotent: skipped if already present)
# ------------------------------------------------------------
# ANGLE pins its own GN revision in DEPS (buildtools/win), independent of
# and older than the one V8's rusty_v8 checkout pins. Reusing V8's GN against
# ANGLE's BUILD.gn produced a real, reproducible failure here (GN's own
# `source_set()` template rejected `generate_modulemap` as an unused
# assignment) -- version-matched GN is not optional.
if [[ -z "$GN_WIN" ]]; then
    GN_WIN="C:\\anglesrc\\buildtools\\win\\gn.exe"
    if [[ ! -f "$SRC_UNIX/buildtools/win/gn.exe" ]]; then
        local_gn_version="$(sed -n "s/.*'version': 'git_revision:\([0-9a-f]*\)'.*/\1/p" "$SRC_UNIX/DEPS" | head -1)"
        [[ -n "$local_gn_version" ]] || { err "cannot find ANGLE's pinned gn revision in DEPS"; exit 1; }
        info "fetching ANGLE's pinned gn (git_revision:$local_gn_version)"
        cat > "$SRC_UNIX/_fetch_gn.ensure" <<EOF
gn/gn/windows-amd64 git_revision:$local_gn_version
EOF
        cat > "$SRC_UNIX/_fetch_gn.bat" <<BAT
@echo off
set DEPOT_TOOLS_UPDATE=0
set HTTPS_PROXY=${PROXY}
set HTTP_PROXY=${PROXY}
call C:\\depot_tools\\cipd.bat ensure -root C:\\anglesrc\\buildtools\\win -ensure-file C:\\anglesrc\\_fetch_gn.ensure
echo EXITCODE=%ERRORLEVEL%
BAT
        ( cd "$SRC_UNIX" && cmd.exe /c "_fetch_gn.bat" )
        rm -f "$SRC_UNIX/_fetch_gn.bat" "$SRC_UNIX/_fetch_gn.ensure"
        [[ -f "$SRC_UNIX/buildtools/win/gn.exe" ]] || { err "gn fetch did not produce buildtools/win/gn.exe"; exit 1; }
    fi
fi

if [[ ! -f "$SRC_UNIX/third_party/llvm-build/Release+Asserts/cr_build_revision" ]]; then
    info "fetching Chromium's pinned clang (gclient runhooks never triggers this for ANGLE, unlike V8's build.rs)"
    cat > "$SRC_UNIX/_fetch_clang.bat" <<EOF
@echo off
set HTTPS_PROXY=$PROXY
set HTTP_PROXY=$PROXY
set PATH=$(win_dirname "$PYTHON_WIN");%PATH%
cd /d $ANGLE_SRC_WIN
python "$ANGLE_SRC_WIN\\tools\\clang\\scripts\\update.py"
echo EXITCODE=%ERRORLEVEL%
EOF
    ( cd "$SRC_UNIX" && cmd.exe /c "_fetch_clang.bat" )
    rm -f "$SRC_UNIX/_fetch_clang.bat"
    [[ -f "$SRC_UNIX/third_party/llvm-build/Release+Asserts/cr_build_revision" ]] \
        || { err "clang fetch did not produce third_party/llvm-build/Release+Asserts/"; exit 1; }
fi

# ------------------------------------------------------------
# Build
# ------------------------------------------------------------
BATCH_UNIX="/mnt/c/migo-win-spike-tmp/migo-build-angle-windows.bat"
mkdir -p "$(dirname "$BATCH_UNIX")"

case "$ARCH" in
    aarch64) VCVARS_BAT="vcvarsamd64_arm64.bat" ;;
    x86_64)  VCVARS_BAT="vcvars64.bat" ;;
esac

PROXY_LINES=""
[[ -n "$PROXY" ]] && PROXY_LINES="set HTTPS_PROXY=${PROXY}
set HTTP_PROXY=${PROXY}"

NUM_JOBS_ARG=""
[[ -n "$NUM_JOBS_WIN" ]] && NUM_JOBS_ARG="-j $NUM_JOBS_WIN"

# Windows paths here use single backslashes, matching gn's own arg-string
# escaping (proven empirically: doubled backslashes were tried first and gn
# parsed them as literal double backslashes in the resulting path, which then
# failed to resolve).
WDK_PATH_WIN="C:\Program Files (x86)\Windows Kits\10"
GN_ARGS="target_cpu=\\\"$GN_CPU\\\" is_debug=false is_component_build=false angle_enable_wgpu=false visual_studio_path=\\\"$VS_ROOT\\\" visual_studio_version=\\\"2022\\\" windows_sdk_version=\\\"$SDK_VERSION\\\" wdk_path=\\\"$WDK_PATH_WIN\\\""
OUT_SUBDIR="out\\\\$PUBLIC_ARCH-release"

cat > "$BATCH_UNIX" <<BAT
@echo off
set "ERRORLEVEL="
set DEPOT_TOOLS_UPDATE=0
set DEPOT_TOOLS_WIN_TOOLCHAIN=0
${PROXY_LINES}
rem This project's own SDK never carries Chromium's internal SDK pin (see
rem engine/third_party/angle-patches/0001's header); MIGO_ANGLE_WIN_SDK_VERSION
rem is that patch's escape hatch, fed the same auto-detected version passed
rem to gn as windows_sdk_version so the two stay consistent.
set MIGO_ANGLE_WIN_SDK_VERSION=${SDK_VERSION}
set PATH=$(win_dirname "$PYTHON_WIN");C:\\anglesrc\\buildtools\\win;$(win_dirname "$NINJA_WIN");C:\\depot_tools;%PATH%
rem Chromium build/vs_toolchain.py probes only %ProgramFiles% for VS2022,
rem while Build Tools commonly installs under %ProgramFiles(x86)%.
rem vs2022_install is that script's own documented override.
set "vs2022_install=${VS_ROOT}"
call "${VS_ROOT}\\VC\\Auxiliary\\Build\\${VCVARS_BAT}" >nul

cd /d ${ANGLE_SRC_WIN} || exit /b 90
gn gen ${OUT_SUBDIR} --args="${GN_ARGS}"
if errorlevel 1 exit /b %errorlevel%
ninja -C ${OUT_SUBDIR} ${NUM_JOBS_ARG} libEGL libGLESv2
set NINJA_EXIT=%errorlevel%
echo === EXIT=%NINJA_EXIT% ===
exit /b %NINJA_EXIT%
BAT

info "building ANGLE ($GN_CPU) in $ANGLE_SRC_WIN (this takes several minutes)"
( cd "$(dirname "$BATCH_UNIX")" && cmd.exe /c "$(basename "$BATCH_UNIX")" )

# ------------------------------------------------------------
# Install artifacts
# ------------------------------------------------------------
GN_OUT_UNIX="$SRC_UNIX/out/$PUBLIC_ARCH-release"
DLL_EGL="$GN_OUT_UNIX/libEGL.dll"
DLL_GLES="$GN_OUT_UNIX/libGLESv2.dll"
IMPLIB_EGL="$GN_OUT_UNIX/libEGL.dll.lib"
IMPLIB_GLES="$GN_OUT_UNIX/libGLESv2.dll.lib"
D3DCOMPILER_SRC="/mnt/c/Program Files (x86)/Windows Kits/10/Redist/D3D/$GN_CPU/d3dcompiler_47.dll"

for f in "$DLL_EGL" "$DLL_GLES" "$IMPLIB_EGL" "$IMPLIB_GLES"; do
    [[ -f "$f" ]] || { err "missing build product: $f"; exit 1; }
done

mkdir -p "$OUT_DIR"
cp "$DLL_EGL" "$OUT_DIR/libEGL.dll"
cp "$DLL_GLES" "$OUT_DIR/libGLESv2.dll"
cp "$IMPLIB_EGL" "$OUT_DIR/libEGL.dll.lib"
cp "$IMPLIB_GLES" "$OUT_DIR/libGLESv2.dll.lib"
cp "$D3DCOMPILER_SRC" "$OUT_DIR/d3dcompiler_47.dll"

ok "libEGL.dll     -> $OUT_DIR/libEGL.dll ($(du -h "$OUT_DIR/libEGL.dll" | cut -f1))"
ok "libGLESv2.dll   -> $OUT_DIR/libGLESv2.dll ($(du -h "$OUT_DIR/libGLESv2.dll" | cut -f1))"
ok "d3dcompiler_47  -> $OUT_DIR/d3dcompiler_47.dll (from Windows SDK Redist/D3D/$GN_CPU)"
ok "ANGLE build complete for $ARCH ($PUBLIC_ARCH)"
