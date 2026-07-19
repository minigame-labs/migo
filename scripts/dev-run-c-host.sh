#!/usr/bin/env bash
# scripts/dev-run-c-host.sh
#
# Build and run examples/c-host: a third-party host written in C that drives the
# engine through the public C ABI only. It links the `capi` staticlib, which is
# why no shared-TLS V8 rebuild is needed yet (CLAUDE.md §10) — a staticlib goes
# into a normal executable.
#
# Usage:
#   scripts/dev-run-c-host.sh [GAME_BUNDLE_DIR] [SECONDS]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE_DIR="$REPO_ROOT/engine"

c_info() { echo -e "\033[0;36m[c-host] $*\033[0m"; }
c_err()  { echo -e "\033[0;31m[c-host] $*\033[0m" >&2; }

GAME_DIR="${1:-$REPO_ROOT/../migo-bench/shells/migo-shell/app/src/main/assets/game}"
SECS="${2:-10}"
CONTENT_ID="c-host-demo"
RUN_ROOT="${MIGO_C_HOST_ROOT:-/tmp/migo-c-host}"

# ---- host V8 + Skia toolchain env (same as scripts/dev-test-host.sh) ----
V8_DIR="${MIGO_HOST_V8_DIR:-$REPO_ROOT/../rusty_v8_src/target/x86_64-unknown-linux-gnu/release/gn_out}"
[[ -f "$V8_DIR/obj/librusty_v8.a" ]] || { c_err "linux-gnu V8 missing: $V8_DIR/obj/librusty_v8.a"; exit 1; }
bash "$SCRIPT_DIR/dev-setup-skia.sh" >/dev/null

export RUSTY_V8_ARCHIVE="$V8_DIR/obj/librusty_v8.a"
export RUSTY_V8_SRC_BINDING_PATH="$V8_DIR/src_binding.rs"
export CC="${CC_HOST:-/usr/bin/clang}"
export CXX="${CXX_HOST:-/usr/bin/clang++}"
export CPATH="$HOME/.local/skia-headers${CPATH:+:$CPATH}"
export LIBRARY_PATH="$HOME/.local/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"
export LD_LIBRARY_PATH="$HOME/.local/lib:/usr/lib/wsl/lib:/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
unset ANDROID_NDK ANDROID_NDK_HOME || true

# ---- deploy the game bundle where the ABI expects installed content ----
CODE_DIR="$RUN_ROOT/files/migo/games/$CONTENT_ID/code"
mkdir -p "$CODE_DIR"
for name in game.json game.js; do
    [[ -f "$GAME_DIR/$name" ]] || { c_err "missing $GAME_DIR/$name"; exit 1; }
    cp "$GAME_DIR/$name" "$CODE_DIR/$name"
done
c_info "deployed '$CONTENT_ID' to $CODE_DIR"

# ---- build the staticlib, then the C host ----
cd "$ENGINE_DIR"
c_info "building capi staticlib ..."
cargo build -p capi --offline

STATIC_LIB="$ENGINE_DIR/target/debug/libmigo_capi.a"
[[ -f "$STATIC_LIB" ]] || { c_err "staticlib not produced: $STATIC_LIB"; exit 1; }

BIN="$ENGINE_DIR/target/debug/migo-c-host"
OBJ="$ENGINE_DIR/target/debug/migo-c-host.o"
c_info "compiling examples/c-host ..."
# Compiled as plain C11 — the example must stay buildable by a C-only host.
"$CC" -std=c11 -Wall -Wextra -O0 -g \
    -I"$REPO_ROOT/include" \
    -c "$REPO_ROOT/examples/c-host/main.c" -o "$OBJ"

# ---- KNOWN GAP: linking a C host against the staticlib is unfinished ----
#
# A Rust staticlib carries the Rust code and its Rust dependencies, but NOT the
# native archives a build script told cargo to link: those directives only apply
# when cargo itself drives the link. Reproducing that link line by hand does not
# work naively either — pulling Skia's archives in explicitly forces
# `skia-bindings`' translation unit to be included whole, which then needs
# symbols (JPEG/PDF/pathops) that no `libskia.a` in `target/` defines, while
# cargo's own link never pulls those objects in at all.
#
# The fix belongs to the packaging slice, which must derive the link line from
# cargo's link data and ship it as pkg-config/CMake instead of asking each host
# to rediscover it. See docs/superpowers/plans/2026-07-18-c-abi-runtime-plan.md.
#
# Until then this script stops after proving the C host *compiles* against the
# public headers, which is what keeps the example honest about the ABI surface.
c_info "compile-only: examples/c-host builds against the public headers"
c_err "linking is not implemented yet (see the KNOWN GAP note in this script);"
c_err "the C ABI is exercised by 'cargo test -p capi' until packaging lands."
exit 0

c_info "running: $BIN $RUN_ROOT/files $CONTENT_ID $SECS"
exec "$BIN" "$RUN_ROOT/files" "$CONTENT_ID" "$SECS"
