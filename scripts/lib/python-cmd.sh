#!/usr/bin/env bash
# Resolves this machine's Python 3 interpreter name.
#
# `python3` is correct on Linux/macOS/WSL. On a native Windows runner --
# including GitHub's windows-latest after actions/setup-python -- only
# `python` is guaranteed: setup-python adds its interpreter to PATH under the
# name `python`, not `python3` (github.com/actions/setup-python#123, #1060).
# Preferring an existing `python3` and falling back to `python` is therefore
# correct cross-platform behavior, not a Windows-specific workaround: it picks
# whichever name this machine actually provides.
#
# This file is sourced, not executed: no `set -euo pipefail` (each caller sets
# its own), no side effects beyond defining the function below.
python_cmd() {
    if command -v python3 >/dev/null 2>&1; then
        echo python3
    elif command -v python >/dev/null 2>&1; then
        echo python
    else
        echo "error: neither python3 nor python found on PATH" >&2
        return 1
    fi
}
