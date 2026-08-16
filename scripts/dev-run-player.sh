#!/usr/bin/env bash
# scripts/dev-run-player.sh
#
# Build + run the headless Linux dev player (engine/tools/player) on
# x86_64-unknown-linux-gnu, rendering a mini-game-shaped bundle offscreen.
#
# Usage:
#   scripts/dev-run-player.sh [GAME_BUNDLE_DIR] [SECONDS]
#
# `platform` is an rlib and the Android cdylib lives in the `android-jni` crate,
# so the player links it directly — no crate-type juggling. (A cdylib is built
# for every target, and on a glibc host the executable-TLS Linux V8 cannot be
# linked into a shared object: `R_X86_64_TPOFF32 ... cannot be used with
# -shared`.)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE_DIR="$REPO_ROOT/engine"

c_info() { echo -e "\033[0;36m[player] $*\033[0m"; }
c_err()  { echo -e "\033[0;31m[player] $*\033[0m" >&2; }

GAME_DIR="${1:-$REPO_ROOT/../migo-bench/shells/migo-shell/app/src/main/assets/game}"
SECS="${2:-8}"
# Anything further (e.g. --window) goes straight to the player binary.
PLAYER_ARGS=("${@:3}")

# ---- host V8 + Skia toolchain env (see scripts/dev-test-host.sh) ----
# shellcheck source=scripts/lib/host-v8.sh
source "$SCRIPT_DIR/lib/host-v8.sh"
host_v8_resolve "$REPO_ROOT" || exit 1
bash "$SCRIPT_DIR/dev-setup-skia.sh" >/dev/null

export RUSTY_V8_ARCHIVE="$HOST_V8_ARCHIVE"
export RUSTY_V8_SRC_BINDING_PATH="$HOST_V8_BINDING"
export CC="${CC_HOST:-/usr/bin/clang}"
export CXX="${CXX_HOST:-/usr/bin/clang++}"
export CPATH="$HOME/.local/skia-headers${CPATH:+:$CPATH}"
export LIBRARY_PATH="$HOME/.local/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"
export LD_LIBRARY_PATH="$HOME/.local/lib:/usr/lib/wsl/lib:/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PATH="$HOME/.local/bin:$PATH"
unset ANDROID_NDK ANDROID_NDK_HOME || true

cd "$ENGINE_DIR"
c_info "building player ..."
cargo build -p migo-player --offline

c_info "running player: game=$GAME_DIR secs=$SECS"
exec ./target/debug/migo-player "$GAME_DIR" "$SECS" ${PLAYER_ARGS[@]+"${PLAYER_ARGS[@]}"}
