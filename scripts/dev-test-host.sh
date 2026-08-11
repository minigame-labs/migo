#!/usr/bin/env bash
# scripts/dev-test-host.sh
#
# Build / test the migo engine natively on x86_64-unknown-linux-gnu (the §1.5
# "minimum compatibility baseline" for the Linux support profile). This is the
# host-side counterpart to scripts/build-android-so.sh.
#
# It wires up the three things a minimal Ubuntu / WSL2 host lacks for a migo
# host build:
#   1. A linux-gnu `librusty_v8.a` + `src_binding.rs` (V8 is NOT rebuilt here;
#      resolved by scripts/lib/host-v8.sh, which defaults to the in-repo fetch
#      that `scripts/fetch-v8-archives.sh x86_64-linux-gnu` produces).
#   2. The system clang/clang++ as CC/CXX (NOT the Android NDK clang, whose
#      libc++ vs system libstdc++-13 <chrono> mismatch breaks the Skia build),
#      and ANDROID_NDK unset so skia-bindings does not pick the NDK toolchain.
#   3. Khronos EGL/GL headers + libfontconfig/libfreetype/libEGL symlinks via
#      scripts/dev-setup-skia.sh (idempotent).
#
# Usage:
#   scripts/dev-test-host.sh [cargo-args...]
#
# Examples:
#   scripts/dev-test-host.sh test -p migo-runtime-v8 --lib          # V8 backend suite
#   scripts/dev-test-host.sh test -p migo-core --no-default-features --features profile-slim
#   scripts/dev-test-host.sh build -p migo-graphics --lib
#
# Env overrides:
#   MIGO_HOST_V8_DIR  directory holding librusty_v8.a + src_binding.rs, either
#                     side by side (what scripts/fetch-v8-archives.sh produces)
#                     or with the archive under obj/ (a rusty_v8 source build).
#                     Defaults to the in-repo fetch; see scripts/lib/host-v8.sh.
#   CC_HOST / CXX_HOST  host C/C++ compiler (default: /usr/bin/clang{,++})
#
# NOTE: every crate here links as an executable (test binaries, the player), so
# the executable-model local-exec V8 archive is fine. Only a cdylib would need a
# linux-gnu V8 built with a shared-compatible TLS model (`R_X86_64_TPOFF32 ...
# cannot be used with -shared`), and the sole cdylib is `android-jni`
# (libmigo.so), which is only ever built for Android.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENGINE_DIR="$REPO_ROOT/engine"

c_info() { echo -e "\033[0;36m[host] $*\033[0m"; }
c_err()  { echo -e "\033[0;31m[host] $*\033[0m" >&2; }

V8_ARCHIVE=""
V8_BINDING=""
# shellcheck source=scripts/lib/host-v8.sh
source "$SCRIPT_DIR/lib/host-v8.sh"
host_v8_resolve "$REPO_ROOT" || exit 1
V8_ARCHIVE="$HOST_V8_ARCHIVE"
V8_BINDING="$HOST_V8_BINDING"

# Host Skia deps (idempotent): EGL/GL headers + .so symlinks + ninja.
c_info "ensuring host Skia deps (scripts/dev-setup-skia.sh)"
bash "$SCRIPT_DIR/dev-setup-skia.sh" >/dev/null

CC_HOST="${CC_HOST:-/usr/bin/clang}"
CXX_HOST="${CXX_HOST:-/usr/bin/clang++}"

export RUSTY_V8_ARCHIVE="$V8_ARCHIVE"
export RUSTY_V8_SRC_BINDING_PATH="$V8_BINDING"
export CC="$CC_HOST"
export CXX="$CXX_HOST"
export CPATH="$HOME/.local/skia-headers${CPATH:+:$CPATH}"
export LIBRARY_PATH="$HOME/.local/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"
export LD_LIBRARY_PATH="$HOME/.local/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PATH="$HOME/.local/bin:$PATH"
# Skia-bindings must not pick the Android NDK toolchain for a host build.
unset ANDROID_NDK ANDROID_NDK_HOME || true

c_info "V8: $V8_ARCHIVE"
c_info "CC=$CC CXX=$CXX"

ARGS=("$@")

# `--probe` answers one question — is this host set up to build the engine
# natively? — and answers it by having run the whole preparation above rather
# than by re-checking a copy of its conditions. A caller that asked separately
# would be a second definition of "usable", and the two would drift.
if [[ ${#ARGS[@]} -eq 1 && "${ARGS[0]}" == "--probe" ]]; then
    c_info "host toolchain is usable"
    exit 0
fi

[[ ${#ARGS[@]} -gt 0 ]] || ARGS=(test -p migo-runtime-v8 --lib --offline)

cd "$ENGINE_DIR"
c_info "cargo ${ARGS[*]}"
exec cargo "${ARGS[@]}"
