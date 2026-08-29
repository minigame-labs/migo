#!/usr/bin/env bash
set -euo pipefail
# Stage the tools and headers the Windows Skia build needs but the toolchain
# does not provide: ninja, and the Khronos EGL / GLES / KHR headers Skia's
# Ganesh GL interface includes. Same set `scripts/dev-setup-skia.sh` fetches on
# Linux, staged where `probe-layer.sh`'s generated batch expects them
# (`WIN_TOOLS_DOS` / `WIN_HEADERS_DOS` in lib.sh).
#
# Idempotent: a present, runnable ninja and a present `EGL/egl.h` are left
# alone. Downloads go through WSL's network, not the Windows side.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

NINJA_VERSION="1.12.1"
TOOLS_UNIX="$(win_to_unix_path "$WIN_TOOLS_DOS")"
HEADERS_UNIX="$(win_to_unix_path "$WIN_HEADERS_DOS")"

mkdir -p "$TOOLS_UNIX" "$HEADERS_UNIX"/{EGL,KHR,GL,GLES2,GLES3}

# The probe runs cargo with CARGO_HOME set to WIN_CARGO_HOME_DOS (short, for
# MAX_PATH), so it does not read the crates.io mirror in the Windows user's
# %USERPROFILE%\.cargo\config.toml. On a network where upstream crates.io
# downloads stall for tens of minutes, that mirror is what makes a cold build
# finish -- so put the same choice in the home this probe uses. Machine-local,
# not repo state; delete the file to go back upstream.
CARGO_HOME_UNIX="$(win_to_unix_path "$WIN_CARGO_HOME_DOS")"
if [[ -f "$CARGO_HOME_UNIX/config.toml" ]]; then
    echo "[stage] $WIN_CARGO_HOME_DOS\\config.toml already present"
else
    mirror_src=""
    for candidate in /mnt/c/Users/*/.cargo/config.toml /mnt/c/Users/*/.cargo/config; do
        if [[ -f "$candidate" ]] && grep -q 'replace-with' "$candidate" 2>/dev/null; then
            mirror_src="$candidate"
            break
        fi
    done
    if [[ -n "$mirror_src" ]]; then
        mkdir -p "$CARGO_HOME_UNIX"
        cp "$mirror_src" "$CARGO_HOME_UNIX/config.toml"
        echo "[stage] copied the crates.io mirror from ${mirror_src#/mnt/c/} into $WIN_CARGO_HOME_DOS"
    else
        echo "[stage] no crates.io mirror found under C:\\Users; a cold Windows build will use upstream (slow on a restricted network)"
    fi
fi

if "$TOOLS_UNIX/ninja.exe" --version >/dev/null 2>&1; then
    echo "[stage] ninja $("$TOOLS_UNIX/ninja.exe" --version) already staged"
else
    echo "[stage] fetching ninja $NINJA_VERSION"
    tmp="$(mktemp -d)"
    curl -sSLf \
        "https://github.com/ninja-build/ninja/releases/download/v${NINJA_VERSION}/ninja-win.zip" \
        -o "$tmp/ninja-win.zip"
    unzip -oq "$tmp/ninja-win.zip" -d "$TOOLS_UNIX"
    chmod +x "$TOOLS_UNIX/ninja.exe"
    rm -rf "$tmp"
    echo "[stage] ninja $("$TOOLS_UNIX/ninja.exe" --version)"
fi

if [[ -f "$HEADERS_UNIX/EGL/egl.h" ]]; then
    echo "[stage] Khronos headers already staged"
else
    echo "[stage] fetching Khronos EGL / GLES headers"
    for h in EGL/egl.h EGL/eglext.h EGL/eglplatform.h KHR/khrplatform.h; do
        curl -sSLf "https://registry.khronos.org/EGL/api/$h" -o "$HEADERS_UNIX/$h"
    done
    for h in GL/glext.h GLES2/gl2.h GLES2/gl2ext.h GLES2/gl2platform.h \
             GLES3/gl3.h GLES3/gl3platform.h; do
        curl -sSLf "https://registry.khronos.org/OpenGL/api/$h" -o "$HEADERS_UNIX/$h"
    done
    echo "[stage] $(find "$HEADERS_UNIX" -name '*.h' | wc -l) headers staged"
fi

# The first cold `cargo check` also downloads a prebuilt Skia binaries tarball
# from github.com. That fetch runs on the Windows side, so it needs a
# Windows-reachable proxy -- `verify-compile.sh` finds one with
# `detect_windows_proxy`. Once `C:\mt` is warm the download never repeats.
proxy="$(detect_windows_proxy)"
if [[ -n "$proxy" ]]; then
    echo "[stage] Windows proxy for the Skia binaries fetch: $proxy"
else
    echo "[stage] no Windows proxy found; a cold Skia binaries download may stall -- set MIGO_WIN_PROXY"
fi

echo "[stage] done -- verify-change.sh's windows:compile lane can now run"
