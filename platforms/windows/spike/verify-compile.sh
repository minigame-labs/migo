#!/usr/bin/env bash
set -euo pipefail
# `verify-change.sh`'s windows:compile lane.
#
# Sync the WSL working tree to the Windows local disk, then `cargo check` the
# crates that carry `cfg(windows)` / MSVC-only code for x86_64-pc-windows-msvc.
# `verification_targets.py` already routes any change under `platform/src/` or
# `capi/src/platform/` here; this is the build that answers it.
#
# `verify-change.sh` only reaches this script after checking the toolchain, the
# Windows V8 import library and the staged Skia deps are all present, so a
# failure here is a real compile failure, not a missing prerequisite.

SPIKE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SPIKE_DIR/lib.sh"

# The Windows V8 import library and its prebuilt bindings, read over the WSL
# UNC path rather than copied: `git clean -xdff` in sync-worktree.sh wipes a
# copy inside the worktree, and the file is 200 MB.
V8_DIR_UNIX="$REPO_ROOT/engine/third_party/rusty_v8/x86_64-pc-windows-msvc"
MIGO_WIN_V8_ARCHIVE="$(wslpath -w "$V8_DIR_UNIX/rusty_v8.lib")"
MIGO_WIN_V8_SRC_BINDING="$(wslpath -w "$V8_DIR_UNIX/src_binding.rs")"
export MIGO_WIN_V8_ARCHIVE MIGO_WIN_V8_SRC_BINDING

# So a cold Skia build can fetch its prebuilt tarball from the Windows side.
# No-op with a warm `C:\mt` (nothing to download) or a caller-set proxy.
proxy="$(detect_windows_proxy)"
if [[ -n "$proxy" ]]; then
    export MIGO_WIN_PROXY="$proxy"
    echo "[verify-compile] using Windows proxy $proxy for the Skia binaries fetch"
fi

bash "$SPIKE_DIR/sync-worktree.sh"

# The crates `verification_targets.py::ANDROID_GATED_CRATES` overlaps with on
# the Windows side: platform's `src/windows/**` and capi's
# `src/platform/windows.rs` are the only `cfg(windows)` engine code. Checking
# platform pulls capi's dependency-free half in anyway, but naming both makes a
# capi-only Windows break visible.
status=0
for package in migo-platform migo-capi; do
    if ! bash "$SPIKE_DIR/probe-layer.sh" "$package"; then
        status=1
    fi
done
exit "$status"
