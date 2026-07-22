#!/usr/bin/env bash
set -euo pipefail
# Put the current HEAD on a Windows local disk for the Windows toolchain to build.
#
# LFS objects are skipped by default: the first probe layers need none of them,
# and the repository's LFS budget is exhausted for reads (uploads still work),
# so a smudge would fail rather than merely be slow. Task 3 copies the one
# archive it needs directly from the WSL-side LFS cache, which never touches
# the network.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

if [[ -d "$WIN_WORKTREE_UNIX/.git" ]]; then
    echo "[sync] updating existing worktree at $WIN_WORKTREE_UNIX"
    GIT_LFS_SKIP_SMUDGE=1 git -C "$WIN_WORKTREE_UNIX" fetch --depth 1 origin HEAD
    GIT_LFS_SKIP_SMUDGE=1 git -C "$WIN_WORKTREE_UNIX" checkout --force FETCH_HEAD
    # --force overwrites tracked files but never removes untracked ones; without
    # this, anything a previous Windows build left behind survives every future
    # sync and can change what `cargo check` resolves.
    git -C "$WIN_WORKTREE_UNIX" clean -xdff
else
    echo "[sync] cloning into $WIN_WORKTREE_UNIX"
    rm -rf "$WIN_WORKTREE_UNIX"
    # file:// forces a real transport; a plain path would hardlink across the
    # WSL/Windows filesystem boundary and fail.
    GIT_LFS_SKIP_SMUDGE=1 git clone --depth 1 --no-local \
        "file://$REPO_ROOT" "$WIN_WORKTREE_UNIX"
fi

# Assigned before echoing: under `set -e`, a failing command substitution
# inside an echo's arguments does not abort the script (the echo itself still
# succeeds), so these would otherwise be status lines that cannot report
# failure.
WORKTREE_HEAD="$(git -C "$WIN_WORKTREE_UNIX" rev-parse --short HEAD)"
# `|| true`: du returns nonzero on an unreadable entry, and under `set -e`
# that would abort *after* a successful sync, making it look like a failure.
WORKTREE_SIZE="$(du -sh "$WIN_WORKTREE_UNIX" 2>/dev/null | cut -f1 || true)"
echo "[sync] HEAD: $WORKTREE_HEAD"
echo "[sync] size: $WORKTREE_SIZE"
