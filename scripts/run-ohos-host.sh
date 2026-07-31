#!/usr/bin/env bash
# =============================================================================
# Install and launch the OpenHarmony host on a connected device or emulator.
#
# Counterpart to scripts/build-ohos-host.sh. Split out because the build needs
# DevEco and the install needs a device, and having one script fail for the
# other's reason makes both harder to read.
#
# hdc, not adb. They are different protocols with different daemons (hdcd vs
# adbd), so `adb devices` showing nothing is the expected result on an
# OpenHarmony target rather than a sign of a broken connection -- and an
# Android phone attached at the same time will show up in adb and not in hdc.
#
# Usage:
#   scripts/run-ohos-host.sh [--no-install] [--shot <file.jpeg>]
#
# Env:
#   DEVECO_HOME       DevEco Studio install (hdc ships inside its SDK)
#   MIGO_OHOS_WIN_DIR Windows-side project directory (default C:\migo-ohos-host)
#   MIGO_OHOS_HDC     explicit path to hdc
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

info() { echo -e "\033[0;36m[ohos-run] $*\033[0m"; }
err()  { echo -e "\033[0;31m[ohos-run] $*\033[0m" >&2; }
ok()   { echo -e "\033[0;32m[ohos-run] $*\033[0m"; }

INSTALL=1
SHOT=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-install) INSTALL=0; shift ;;
        --shot) SHOT="$2"; shift 2 ;;
        *) err "unknown argument: $1"; exit 2 ;;
    esac
done

DEVECO_HOME="${DEVECO_HOME:-/mnt/c/Program Files/Huawei/DevEco Studio}"
HDC="${MIGO_OHOS_HDC:-$DEVECO_HOME/sdk/default/openharmony/toolchains/hdc.exe}"
if [[ ! -f "$HDC" ]]; then
    err "hdc not found at $HDC"
    err "set MIGO_OHOS_HDC, or DEVECO_HOME to the DevEco Studio installation"
    exit 1
fi

# hdc.exe is a Windows binary reached through WSL interop, and cmd-launched
# processes refuse a UNC working directory. Everything below therefore runs from
# a local path and passes absolute Windows paths explicitly.
cd /tmp

TARGETS="$("$HDC" list targets 2>/dev/null | tr -d '\r' | grep -v '^$' || true)"
if [[ -z "$TARGETS" || "$TARGETS" == *"Empty"* ]]; then
    err "no OpenHarmony target connected (hdc list targets is empty)"
    err "start the emulator in DevEco Device Manager, or connect a device"
    exit 1
fi
info "target: $(echo "$TARGETS" | head -1)"

WIN_DIR="${MIGO_OHOS_WIN_DIR:-C:\\migo-ohos-host}"
HAP_WIN="$WIN_DIR\\entry\\build\\default\\outputs\\default\\entry-default-unsigned.hap"
HAP_WSL="$(wslpath -u "$HAP_WIN")"

if [[ $INSTALL -eq 1 ]]; then
    [[ -f "$HAP_WSL" ]] || {
        err "no HAP at $HAP_WSL"
        err "build it first: scripts/build-ohos-host.sh"
        exit 1
    }
    info "installing $(stat -c %s "$HAP_WSL") bytes"
    # bm install takes a directory, not a file: it installs every HAP inside.
    # A stale HAP left in that directory would be installed alongside the new
    # one, so the directory is recreated rather than reused.
    "$HDC" shell "rm -rf /data/local/tmp/migohap; mkdir -p /data/local/tmp/migohap" >/dev/null
    "$HDC" file send "$HAP_WIN" /data/local/tmp/migohap/entry.hap >/dev/null
    INSTALL_OUT="$("$HDC" shell "bm install -p /data/local/tmp/migohap" 2>&1 | tr -d '\r')"
    case "$INSTALL_OUT" in
        *successfully*) ok "installed" ;;
        *) err "install failed: $INSTALL_OUT"; exit 1 ;;
    esac
fi

# Read the bundle name off the device rather than hardcoding it: the value lives
# in AppScope/app.json5, and a copy here would go stale without failing.
BUNDLE="$("$HDC" shell "bm dump -a" 2>&1 | tr -d '\r \t' | grep '^com\.migo\.' | head -1)"
if [[ -z "$BUNDLE" ]]; then
    err "no com.migo.* bundle is installed on the target"
    exit 1
fi

info "launching $BUNDLE"
"$HDC" shell "aa force-stop $BUNDLE" >/dev/null 2>&1 || true
# Clearing the log before launch is what makes the diagnostics below belong to
# this run; without it a previous run's lines are indistinguishable.
"$HDC" shell "hilog -r" >/dev/null 2>&1 || true
LAUNCH="$("$HDC" shell "aa start -a EntryAbility -b $BUNDLE" 2>&1 | tr -d '\r')"
case "$LAUNCH" in
    *successfully*) ok "launched" ;;
    *) err "launch failed: $LAUNCH"; exit 1 ;;
esac

sleep 6
info "engine log:"
"$HDC" shell "hilog -x" 2>/dev/null | tr -d '\r' \
    | grep -E "migo-host|JSAPP: \[migo\]|touchprobe" | tail -20 || true

if [[ -n "$SHOT" ]]; then
    # Pixels are the only evidence that survives a host whose every callback
    # reports success; this project has shipped an all-green black screen before.
    SHOT_WIN="$WIN_DIR\\$(basename "$SHOT")"
    "$HDC" shell "snapshot_display -f /data/local/tmp/migo-shot.jpeg" >/dev/null 2>&1
    "$HDC" file recv /data/local/tmp/migo-shot.jpeg "$SHOT_WIN" >/dev/null
    cp "$(wslpath -u "$SHOT_WIN")" "$SHOT"
    ok "screenshot: $SHOT"
fi
