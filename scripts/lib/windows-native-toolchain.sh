#!/usr/bin/env bash
# One correctness fix, needed by any script that runs `cargo` for the MSVC
# target under Git for Windows' bash: build-windows-sdk-native.sh,
# test-windows-sdk-contract.sh (native branch), and package-sdk.sh (which
# builds tools/artifact-manifest with `cargo`, on every platform it packages,
# Windows included).
#
# This file is sourced, not executed: no `set -euo pipefail` (each caller sets
# its own), no side effects beyond defining the function below -- it must be
# safe to source from a script that also runs on Linux/Android/OpenHarmony,
# where nothing here is ever called.
#
# Git for Windows' bash ships its own `usr/bin/link.exe` -- a GNU coreutils
# hardlink tool, unrelated to MSVC's linker of the same name -- and its
# runtime prepends usr/bin to PATH on every fresh invocation, ahead of
# whatever an outer cmd.exe, a prior CI step (ilammy/msvc-dev-cmd's own
# GITHUB_PATH additions included), or a parent shell already set. This is not
# a one-time, job-level problem: every `shell: bash` step in a Windows CI job
# is its own fresh Git-Bash process, so the shadowing recurs each time,
# unfixed at the job level. rustc invokes the MSVC target's linker by the bare
# name "link.exe", so whichever one PATH resolves first wins; unfixed, it
# silently resolves to the wrong tool and every link fails with a cryptic
# "extra operand" error instead of a missing-linker message.
#
# `cl.exe` has no such collision (not a POSIX/GNU utility name), so it is the
# reliable anchor: MSVC's real link.exe lives in the same directory, and
# prepending that directory to PATH makes it win over Git's own copy.
windows_native_ensure_msvc_link_wins() {
    local cl_dir
    cl_dir="$(command -v cl.exe 2>/dev/null)" || {
        echo "[win-toolchain] cl.exe not found -- vcvars was not loaded before this script ran" >&2
        return 1
    }
    export PATH="$(dirname "$cl_dir"):$PATH"
}
