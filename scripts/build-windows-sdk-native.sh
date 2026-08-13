#!/usr/bin/env bash
# Build the Windows x64 SDK, running natively on Windows -- no WSL, no
# wslpath/cmd.exe crossing. This is the CI entry point (GitHub's windows-latest
# runner, via Git Bash), and works equally on a plain Windows box with Git for
# Windows and Visual Studio Build Tools installed, no WSL required.
#
# scripts/build-windows-sdk.sh is the other entry point, for this project's own
# WSL2 development machine, where the toolchain lives on a native Windows disk
# reached by crossing a WSL/Windows boundary that simply does not exist here: a
# checkout on a native runner already sits on NTFS. The two scripts do not share
# that boundary-crossing code because there is no boundary to share it over;
# they DO share the environment-independent packaging tail -- see
# scripts/lib/windows-sdk-package.sh for what lives there and why.
#
# Precondition: link.exe and cl.exe already on PATH, i.e. vcvars64.bat has
# already run in this shell (locally) or ilammy/msvc-dev-cmd already ran as a
# prior CI step. This script does not probe vswhere or locate Visual Studio
# itself -- that discovery belongs to whatever composed this environment, once,
# declaratively, not to every script that needs the result.
#
# Usage: scripts/build-windows-sdk-native.sh [--prefix DIR]
#
# Optional env:
#   MIGO_WIN_ANGLE_DIR   overrides the default ANGLE runtime location
#                         (engine/third_party/angle-windows)
#   CARGO_TARGET_DIR     overrides the default short target dir (see below)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=scripts/lib/windows-sdk-package.sh
source "$SCRIPT_DIR/lib/windows-sdk-package.sh"
# shellcheck source=scripts/lib/release-version.sh
source "$SCRIPT_DIR/lib/release-version.sh"

# MSYS/Git-Bash treats an argv token starting with `/` as a candidate POSIX
# path and rewrites it before exec -- which is exactly the shape of every
# link.exe flag this script passes (/DLL, /DEF:..., /OUT:..., /OPT:REF). Unset,
# `/OPT:REF` has been observed to arrive at link.exe as a mangled filesystem
# path instead of a flag. This disables that rewriting for every process this
# script spawns; every path link.exe or cargo needs is instead converted
# explicitly, below, via `cygpath -w`, so nothing here depends on the automatic
# behavior this turns off.
export MSYS_NO_PATHCONV=1

TRIPLE="x86_64-pc-windows-msvc"
PREFIX="$REPO_ROOT/dist/migo-windows-x86_64"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --prefix) PREFIX="$2"; shift 2 ;;
        *) echo "[win-sdk-native] unknown argument: $1" >&2; exit 2 ;;
    esac
done

V8_DIR="$REPO_ROOT/engine/third_party/rusty_v8/$TRIPLE"
ANGLE_DIR="${MIGO_WIN_ANGLE_DIR:-$REPO_ROOT/engine/third_party/angle-windows}"

# ---- Preconditions ---------------------------------------------------------
for f in rusty_v8.dll rusty_v8.dll.lib src_binding.rs; do
    [[ -f "$V8_DIR/$f" ]] || {
        echo "[win-sdk-native] missing Windows V8 artifact: $V8_DIR/$f" >&2
        echo "[win-sdk-native] fetch with: bash scripts/fetch-v8-archives.sh $TRIPLE" >&2
        exit 1
    }
done
for f in libEGL.dll libGLESv2.dll d3dcompiler_47.dll; do
    [[ -f "$ANGLE_DIR/$f" ]] || {
        echo "[win-sdk-native] missing ANGLE runtime DLL: $ANGLE_DIR/$f" >&2
        echo "[win-sdk-native] fetch with: bash scripts/fetch-windows-angle.sh" >&2
        exit 1
    }
done
if ! { command -v link.exe >/dev/null 2>&1 && command -v cl.exe >/dev/null 2>&1; }; then
    echo "[win-sdk-native] link.exe/cl.exe not on PATH -- run vcvars64.bat first" >&2
    echo "[win-sdk-native] (locally) or ensure ilammy/msvc-dev-cmd ran before this step (CI)." >&2
    exit 1
fi
command -v cygpath >/dev/null 2>&1 || {
    echo "[win-sdk-native] cygpath not found -- this script needs Git for Windows' bash" >&2
    exit 1
}

to_dos() { cygpath -w "$1"; }

VERSION="$(read_release_version "$REPO_ROOT")"

# Short target dir: a from-source Skia fallback build invokes ninja, which
# calls the ANSI GetFullPathNameA and is capped at MAX_PATH regardless of the
# LongPathsEnabled registry setting. Skia's deepest headers (harfbuzz's
# OT/Layout/GSUB tree) run ~165 characters on their own, so every character
# spent on this prefix comes off that budget. Proven necessary, not
# precautionary: platforms/windows/spike/lib.sh records `C:\migo-win-target`
# (18 characters) failing at ninja step 1005 for exactly this reason, which is
# why that default is `C:\mt` rather than something readable.
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-C:\\mt}"
export CARGO_TARGET_DIR
CARGO_TARGET_DIR_UNIX="$(cygpath -u "$CARGO_TARGET_DIR")"

V8_DIR_DOS="$(to_dos "$V8_DIR")"
export RUSTY_V8_ARCHIVE="$V8_DIR_DOS\\rusty_v8.dll.lib"
export RUSTY_V8_SRC_BINDING_PATH="$V8_DIR_DOS\\src_binding.rs"

# ---- Generate the export allowlist (.def) from the headers ----------------
DEF_UNIX="$(mktemp -d)/migo.def"
windows_sdk_generate_def "$REPO_ROOT/include/migo" "$DEF_UNIX"
DEF_DOS="$(to_dos "$DEF_UNIX")"

# ---- Stage 1: build the capi staticlib, capture native-static-libs --------
cd "$REPO_ROOT/engine"
info "building capi staticlib (release)"
cargo build -p migo-capi --release --target "$TRIPLE"

STATICLIB_UNIX="$CARGO_TARGET_DIR_UNIX/$TRIPLE/release/migo_capi.lib"
[[ -f "$STATICLIB_UNIX" ]] || { echo "[win-sdk-native] staticlib not found: $STATICLIB_UNIX" >&2; exit 1; }

info "capturing native-static-libs"
NATIVE_LIBS_FILE="$(mktemp)"
# CARGO_TERM_COLOR=never: forced color wraps the note in ANSI codes whose
# trailing reset glues onto the last -l token and corrupts the parsed line --
# see build-android-sdk.sh for the reproduction that found this originally.
CARGO_TERM_COLOR=never cargo rustc -p migo-capi --lib --release --target "$TRIPLE" \
    --crate-type staticlib -- --print native-static-libs > "$NATIVE_LIBS_FILE" 2>&1
NATIVE_LIBS_LINE="$(grep 'native-static-libs:' "$NATIVE_LIBS_FILE" || true)"
[[ -n "$NATIVE_LIBS_LINE" ]] || {
    echo "[win-sdk-native] no native-static-libs line found; full output:" >&2
    cat "$NATIVE_LIBS_FILE" >&2
    exit 1
}
NATIVE_LIBS="${NATIVE_LIBS_LINE#*native-static-libs: }"

# ---- Discover extra link search dirs (skia-bindings, windows-targets) -----
# link.exe resolves the bare library names cargo reports through LIB alone --
# vcvars sets LIB for the system libs, but skia-bindings and windows-targets
# stage theirs in the build output / registry, on neither PATH nor LIB.
# CALLED AFTER THE BUILD: these directories are produced BY stage 1, so
# scanning for them beforehand would describe the previous build, or nothing
# on a cold target. Returns non-zero when skia's directory is not there, so an
# absence is distinguishable from an empty scan rather than silently handing
# link.exe a partial LIB and a bare `LNK1181: cannot open input file
# 'skparagraph.lib'` that reads like a corrupt build.
discover_link_search_dirs() {
    local skia extra=() cargo_home_unix d
    skia="$(find "$CARGO_TARGET_DIR_UNIX/$TRIPLE/release/build" \
        -path '*skia-bindings-*/out/skia/skparagraph.lib' 2>/dev/null | head -1)"
    [[ -n "$skia" ]] || return 1
    extra+=("$(to_dos "$(dirname "$skia")")")
    [[ -d "$CARGO_TARGET_DIR_UNIX/$TRIPLE/release/gn_out/obj" ]] \
        && extra+=("$(to_dos "$CARGO_TARGET_DIR_UNIX/$TRIPLE/release/gn_out/obj")")
    cargo_home_unix="$(cygpath -u "${CARGO_HOME:-$HOME/.cargo}")"
    while IFS= read -r d; do extra+=("$(to_dos "$d")"); done \
        < <(find "$cargo_home_unix/registry" -path '*windows_x86_64_msvc-*/lib' -type d 2>/dev/null)
    (IFS=';'; printf '%s' "${extra[*]}")
}

EXTRA_LIB_DOS="$(discover_link_search_dirs)" || {
    echo "[win-sdk-native] the staticlib built but skia-bindings' output directory was not found under" >&2
    echo "[win-sdk-native]   $CARGO_TARGET_DIR_UNIX/$TRIPLE/release/build/*skia-bindings-*/out/skia/" >&2
    exit 1
}
info "link search dirs discovered: $(awk -F';' '{print NF}' <<<"$EXTRA_LIB_DOS")"

# ---- Stage 2: link migo.dll -------------------------------------------------
OUT_UNIX="$(mktemp -d)"
STATICLIB_DOS="$(to_dos "$STATICLIB_UNIX")"
OUT_DOS="$(to_dos "$OUT_UNIX")"
export LIB="$EXTRA_LIB_DOS;${LIB:-}"

info "linking migo.dll"
# shellcheck disable=SC2086  # NATIVE_LIBS is a cargo-reported, space-separated
# token list and must word-split into separate link.exe arguments.
link /NOLOGO /DLL "/DEF:$DEF_DOS" \
    "/OUT:$OUT_DOS\\migo.dll" \
    "/IMPLIB:$OUT_DOS\\migo.lib" \
    /OPT:REF /OPT:ICF \
    "$STATICLIB_DOS" \
    "$RUSTY_V8_ARCHIVE" \
    $NATIVE_LIBS

[[ -f "$OUT_UNIX/migo.dll" ]] || { echo "[win-sdk-native] link produced no migo.dll" >&2; exit 1; }
[[ -f "$OUT_UNIX/migo.lib" ]] || { echo "[win-sdk-native] link produced no import lib migo.lib" >&2; exit 1; }
info "linked: migo.dll ($(stat -c %s "$OUT_UNIX/migo.dll") bytes) + import lib migo.lib"

# ---- Stage the package, CMake package, and manifest ------------------------
windows_sdk_stage_package "$PREFIX" "$REPO_ROOT/include/migo" \
    "$OUT_UNIX/migo.lib" "$OUT_UNIX/migo.dll" "$V8_DIR/rusty_v8.dll" "$ANGLE_DIR"
windows_sdk_write_cmake_package "$PREFIX" "$VERSION"
info "writing the package manifest"
windows_sdk_write_manifest "$PREFIX" "$VERSION"

# Deliberately does not self-invoke test-windows-sdk-contract.sh: the CI job
# runs it as its own explicit step (see release.yml's release-windows job) so
# a failure shows up as its own named check in the Actions log rather than
# buried in this script's own output. build-windows-sdk.sh, the WSL entry
# point, still self-invokes it for local one-command convenience.
info "NOTE: NuGet packaging is the remaining packaging step; this stages a"
info "      linkable, runnable migo.dll + headers + CMake package."
