#!/usr/bin/env bash
# Does GTK 4 still lack native child surfaces?
#
# The roadmap lists a GTK 4 Host Kit as an increment that only needs doing.
# It does not: GTK 4 gives a GdkSurface to widgets implementing GtkNative --
# the toplevel and popovers -- and removed GtkSocket/GtkPlug, so there is no
# public way to place a native target Migo can present into inside an App's
# layout. The only alternative on offer, presenting into the toplevel's own
# surface, is the child-window overlay the architecture forbids: it ignores
# layout, clipping and z-order.
#
# This runs the probe that establishes that, so the claim stays evidence rather
# than a reading of the documentation, and so the day GTK changes its answer
# somebody finds out. A failure here is not a regression in Migo -- it is the
# signal that a GTK Host Kit may have become possible.
#
# GTK 4 development files are not on a stock CI runner, so an absent toolchain
# reports SKIPPED loudly on stderr rather than passing quietly. Set
# MIGO_REQUIRE_GTK4_PROBE=1 to turn that skip into a failure.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROBE_SOURCE="$ROOT_DIR/platforms/linux/host-kit/probes/gtk4_native_surface_probe.c"

fail() {
    printf 'GTK 4 surface capability: FAIL: %s\n' "$*" >&2
    exit 1
}

skip() {
    printf 'GTK 4 surface capability: SKIPPED: %s\n' "$*" >&2
    if [[ "${MIGO_REQUIRE_GTK4_PROBE:-0}" == "1" ]]; then
        fail "MIGO_REQUIRE_GTK4_PROBE=1 but the probe could not run"
    fi
    exit 0
}

[[ -f "$PROBE_SOURCE" ]] || fail "missing ${PROBE_SOURCE#"$ROOT_DIR"/}"

command -v pkg-config >/dev/null 2>&1 || skip "pkg-config is not installed"
pkg-config --exists gtk4 || skip "gtk4 development files are not installed (libgtk-4-dev)"
command -v cc >/dev/null 2>&1 || skip "no C compiler"
command -v xvfb-run >/dev/null 2>&1 || skip "xvfb-run is not installed"

BUILD_DIR="$(mktemp -d "${TMPDIR:-/tmp}/migo-gtk4-probe.XXXXXX")"
cleanup() {
    case "$BUILD_DIR" in
        "${TMPDIR:-/tmp}"/migo-gtk4-probe.*) rm -rf -- "$BUILD_DIR" ;;
        *) printf 'Refusing to remove unexpected build directory: %s\n' "$BUILD_DIR" >&2 ;;
    esac
}
trap cleanup EXIT

# shellcheck disable=SC2046 # pkg-config emits separate flags on purpose.
cc "$PROBE_SOURCE" -o "$BUILD_DIR/probe" $(pkg-config --cflags --libs gtk4) ||
    fail "the probe did not compile"

set +e
# X11 explicitly: the question is about a native X11 child, and a developer
# desktop exporting WAYLAND_DISPLAY would otherwise answer a different one.
GDK_BACKEND=x11 xvfb-run -a env -u WAYLAND_DISPLAY "$BUILD_DIR/probe"
PROBE_STATUS=$?
set -e

case "$PROBE_STATUS" in
    0) printf 'GTK 4 surface capability: PASS (no native child surface, so Direct Surface stays blocked)\n' ;;
    1) fail "a GTK 4 child widget now owns a native surface; re-read docs/multiplatform-architecture.md" ;;
    *) fail "the probe could not answer (exit $PROBE_STATUS)" ;;
esac
