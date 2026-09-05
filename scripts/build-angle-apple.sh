#!/usr/bin/env bash
# =============================================================================
# Build ANGLE (libEGL.dylib + libGLESv2.dylib) for Apple platforms from source.
# Location: scripts/build-angle-apple.sh
#
# WHY THIS EXISTS AT ALL:
#   .github/workflows/apple-sdk.yml asked rustc, on 2026-09-05, what a non-Rust
#   consumer must link alongside the engine on each Apple platform:
#
#     macOS  -lc++ -framework ApplicationServices -framework OpenGL -liconv ...
#     iOS    -lc++ -framework CoreFoundation -framework CoreGraphics
#            -framework CoreText -framework ImageIO
#            -framework MobileCoreServices -framework UIKit -liconv ...
#
#   There is no GL framework in the iOS list, because there is no GL framework
#   on iOS. Skia is configured for its GL backend; macOS satisfies that by
#   linking the legacy `OpenGL` framework, and iOS has nothing to link. ANGLE
#   over Metal is what fills that gap, and it is also why "just use system GL
#   for a macOS presenter first" buys iOS nothing -- that shortcut links a thing
#   iOS does not have.
#
#   The Windows side of this repository already has the shape being copied here:
#   scripts/build-angle-windows.sh, contracts/artifact-manifest/windows-angle.lock.json
#   and scripts/fetch-windows-angle.sh.
#
# WHY SHARED LIBRARIES AND NOT ANGLE'S STATIC TARGETS.
#   Not a preference. Every presenter in this repository resolves EGL at RUNTIME
#   and then hands `eglGetProcAddress` to `glow::Context::from_loader_function`:
#
#     crates/platform/src/android/presenter.rs   libEGL.so
#     crates/platform/src/linux/presenter.rs     libEGL.so.1
#     crates/platform/src/ohos/presenter.rs      libEGL.so
#     crates/platform/src/windows/presenter.rs   libEGL.dll   (ANGLE)
#
#   ANGLE does publish `//:angle_static` (libEGL_static + libGLESv2_static), and
#   linking that instead would mean the Apple presenter alone resolved GL at link
#   time while the other four resolved it at load time. The dynamic form is the
#   one the engine already knows how to consume, so it is the one built here.
#
# WHAT IS MEASURED RATHER THAN ASSUMED, and where the measurement came from.
#   .github/workflows/apple-angle-probe.yml (PR #185, run 33948370655) ran a
#   checkout and three `gn gen`s on a free `macos-15` runner before any of this
#   was written, because blind-writing a build script for a checkout nobody has
#   performed is the anti-pattern the neighbouring Apple lanes were fixed for six
#   times in one round. What it established:
#
#     - The image starts with 42 GiB free; `fetch --no-history angle` with
#       target_os limited to mac and ios costs 12 GB and 233 s, leaving 29 GiB.
#       "A hosted macOS runner is too small for this" was inherited from the x86
#       image and is false.
#     - `fetch angle` names its solution `.`, so the checkout IS the directory
#       `fetch` was run in -- unlike `fetch chromium`, which creates `src/`. The
#       probe's first run assumed the chromium shape and died after a
#       four-minute fetch. The fix is not a different guess: `.gclient` says
#       which, so `.gclient` is what this script reads.
#     - `target_os="ios"` alone fails at gn import time; it must be paired with
#       `target_environment` (device or simulator). That is why iOS is two
#       configurations, and it happens to be the same split an xcframework
#       already forces -- it refuses a fat archive mixing device and simulator.
#     - One `ninja libEGL libGLESv2` for iOS device arm64 is 334 s and 100 MB.
#
# WHAT THE SLICES ARE, AND WHY THEY ARE NOT LISTED HERE.
#   They are asked of `scripts/build-apple-sdk.sh --print-slices`. The ANGLE
#   xcframework and the engine xcframework are linked into the same application,
#   so their slice sets have to match; if this script kept its own copy of the
#   list, the two could drift and the failure would land in a consumer's link
#   step naming neither script. Five Rust triples across the three platform
#   groups, so five gn configurations -- the probe measured three because three
#   was enough to answer the question it was asking.
#
# WHAT THE REVISION AND THE gn ARGUMENTS ARE, AND WHY THEY ARE NOT HERE EITHER.
#   contracts/artifact-manifest/apple-angle.lock.json. The pin is the identity of
#   what gets built; a copy in this file would be the one that silently wins on
#   the build machine.
#
# Usage:
#   scripts/build-angle-apple.sh --fetch [--source DIR] [--revision SHA]
#   scripts/build-angle-apple.sh --platform <ios|ios-simulator|macos> [--source DIR]
#   scripts/build-angle-apple.sh --xcframework
#   scripts/build-angle-apple.sh --print-gn-args <rust-target-triple>
#   scripts/build-angle-apple.sh --check
#
#   --fetch          Create or update the ANGLE checkout at --source and put it
#                    on the pinned revision. Needs depot_tools on PATH.
#   --platform       gn gen + ninja for every slice of that platform group, then
#                    lipo them into engine/third_party/angle-apple-<platform>/.
#                    macOS only.
#   --xcframework    Assemble the installed platforms into two xcframeworks in
#                    platforms/apple/Frameworks/. macOS only. Two, not one:
#                    `xcodebuild -create-xcframework` holds one library per
#                    platform, and this ships two.
#   --print-gn-args  Print the full gn argument string for one Rust target
#                    triple and exit, on any host. It is how the contract gate
#                    checks this script's real behaviour instead of grepping it,
#                    and it fails closed on a triple with no mapping.
#   --check          Report readiness and build nothing.
#
# bash 3.2: macOS ships it as /bin/bash and this script runs there. No mapfile,
# no associative arrays, no ${v,,}, and every array expansion guarded --
# scripts/test-macos-bash32-contract.sh derives its scope from the workflows, so
# the lane that runs this script puts it in that gate's scope automatically.
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOCK="$REPO_ROOT/contracts/artifact-manifest/apple-angle.lock.json"
SDK_SCRIPT="$REPO_ROOT/scripts/build-apple-sdk.sh"
INSTALL_ROOT="$REPO_ROOT/engine/third_party"
FRAMEWORKS_DIR="$REPO_ROOT/platforms/apple/Frameworks"

BUILD_ROOT="${MIGO_ANGLE_APPLE_BUILD_ROOT:-/tmp/migo-angle-apple}"
SOURCE="${MIGO_ANGLE_APPLE_SRC:-$BUILD_ROOT/src}"

MODE=""
PLATFORM=""
PRINT_TRIPLE=""
REVISION=""

err()  { printf '\033[0;31m[angle-apple] %s\033[0m\n' "$*" >&2; }
ok()   { printf '\033[0;32m[angle-apple] %s\033[0m\n' "$*"; }
info() { printf '\033[0;36m[angle-apple] %s\033[0m\n' "$*"; }

usage() {
    sed -n '/^# Usage:/,/^# ===/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --fetch)          MODE="fetch"; shift ;;
        --platform)       MODE="build"; PLATFORM="${2:-}"; shift 2 ;;
        --xcframework)    MODE="xcframework"; shift ;;
        --print-gn-args)  MODE="print-gn-args"; PRINT_TRIPLE="${2:-}"; shift 2 ;;
        --check)          MODE="check"; shift ;;
        --source)         SOURCE="${2:-}"; shift 2 ;;
        --revision)       REVISION="${2:-}"; shift 2 ;;
        -h|--help)        usage; exit 0 ;;
        *)                err "unknown argument: $1"; usage >&2; exit 2 ;;
    esac
done

[ -n "$MODE" ] || { err "one of --fetch, --platform, --xcframework, --print-gn-args or --check is required"; usage >&2; exit 2; }
[ -f "$LOCK" ] || { err "pin not found: $LOCK"; exit 1; }

# ---------------------------------------------------------------------------
# What the pin says
# ---------------------------------------------------------------------------

# One reader, one place a key name is spelled. `python3 - "$LOCK" key` rather
# than a grep: a JSON file read with a regular expression is a file read
# incorrectly on the day somebody reformats it.
lock_value() {
    python3 - "$LOCK" "$1" <<'PY'
import json
import sys

path, dotted = sys.argv[1], sys.argv[2]
with open(path, encoding="utf-8") as handle:
    node = json.load(handle)
for key in dotted.split("."):
    if not isinstance(node, dict) or key not in node:
        print(f"{path}: no such key: {dotted}", file=sys.stderr)
        raise SystemExit(1)
    node = node[key]
if isinstance(node, list):
    print(" ".join(str(item) for item in node))
else:
    print(node)
PY
}

PINNED_REVISION="$(lock_value source.angle_revision)"
GCLIENT_TARGET_OS="$(lock_value source.gclient_target_os)"
GN_ARGS_COMMON="$(lock_value source.gn_args_common)"
NINJA_TARGETS="$(lock_value source.ninja_targets)"
ANGLE_REPOSITORY="$(lock_value source.repository)"
[ -n "$REVISION" ] || REVISION="$PINNED_REVISION"

# ---------------------------------------------------------------------------
# Rust target triple -> gn configuration
# ---------------------------------------------------------------------------

# The one thing in this file that is a mapping rather than a lookup, because
# there is nowhere else it exists: it is the correspondence between Rust's Apple
# triples and Chromium's build configuration, and neither project publishes it.
#
# Total over the triples build-apple-sdk.sh reports and fail-closed outside them
# -- scripts/test-apple-angle-recipe-contract.sh checks both halves. A default
# branch that guessed would produce a build for the wrong environment, and the
# artifact would be indistinguishable from a right one until an app failed to
# launch.
#
# `x86_64-apple-ios` is a simulator target: there is no Intel iOS device, which
# is why the pair (simulator, x64) and not (device, x64) is the correct reading
# of it.
gn_args_for_triple() {
    triple="$1"
    ios_target=""
    mac_target=""
    case "$triple" in
        aarch64-apple-ios|aarch64-apple-ios-sim|x86_64-apple-ios)
            ios_target="$(deployment_target ios)" || return 1 ;;
        aarch64-apple-darwin|x86_64-apple-darwin)
            mac_target="$(deployment_target macos)" || return 1 ;;
    esac
    case "$triple" in
        aarch64-apple-ios)
            printf '%s target_os="ios" target_environment="device" target_cpu="arm64" ios_enable_code_signing=false ios_deployment_target="%s"' \
                "$GN_ARGS_COMMON" "$ios_target" ;;
        aarch64-apple-ios-sim)
            printf '%s target_os="ios" target_environment="simulator" target_cpu="arm64" ios_enable_code_signing=false ios_deployment_target="%s"' \
                "$GN_ARGS_COMMON" "$ios_target" ;;
        x86_64-apple-ios)
            printf '%s target_os="ios" target_environment="simulator" target_cpu="x64" ios_enable_code_signing=false ios_deployment_target="%s"' \
                "$GN_ARGS_COMMON" "$ios_target" ;;
        aarch64-apple-darwin)
            printf '%s target_os="mac" target_cpu="arm64" mac_deployment_target="%s"' \
                "$GN_ARGS_COMMON" "$mac_target" ;;
        x86_64-apple-darwin)
            printf '%s target_os="mac" target_cpu="x64" mac_deployment_target="%s"' \
                "$GN_ARGS_COMMON" "$mac_target" ;;
        *)
            err "no ANGLE configuration for Rust target: $triple"
            err "Every slice scripts/build-apple-sdk.sh --print-slices reports needs one"
            err "here, or ANGLE would be built for a different set of architectures than"
            err "the engine and the two xcframeworks could not be linked into one app."
            return 1 ;;
    esac
}

# The deployment target, asked of the script that owns
# contracts/apple/deployment-floor.json rather than read from the contract
# again here. Two readers of one JSON file is two places a key name can be
# spelled wrong; one reader with two callers is one.
deployment_target() {
    bash "$SDK_SCRIPT" --print-deployment-target "$1"
}

slices_for_platform() {
    bash "$SDK_SCRIPT" --print-slices "$1"
}

if [ "$MODE" = "print-gn-args" ]; then
    [ -n "$PRINT_TRIPLE" ] || { err "--print-gn-args needs a Rust target triple"; exit 2; }
    gn_args_for_triple "$PRINT_TRIPLE" || exit 1
    printf '\n'
    exit 0
fi

# ---------------------------------------------------------------------------
# Where the checkout is
# ---------------------------------------------------------------------------

# Read from .gclient, never guessed. See the header: `fetch angle` names its
# solution `.`, and the probe's first run lost a four-minute fetch to assuming
# otherwise. Reading the file also means a shape change upstream keeps working.
angle_root() {
    [ -f "$SOURCE/.gclient" ] || { err "not a gclient checkout: $SOURCE (no .gclient)"; return 1; }
    root="$(python3 - "$SOURCE/.gclient" <<'PY'
import pathlib
import sys

source = pathlib.Path(sys.argv[1]).read_text()
namespace = {}
exec(compile(source, ".gclient", "exec"), namespace)
solutions = namespace["solutions"]
if len(solutions) != 1:
    raise SystemExit(
        f".gclient declares {len(solutions)} solutions; this recipe knows how to locate one"
    )
print(solutions[0]["name"])
PY
)" || return 1
    (cd "$SOURCE/$root" && pwd)
}

# ---------------------------------------------------------------------------
# Readiness
# ---------------------------------------------------------------------------

check_ready() {
    failures=0
    for tool in python3 git; do
        command -v "$tool" >/dev/null 2>&1 || { err "not on PATH: $tool"; failures=$((failures + 1)); }
    done
    case "$MODE" in
        fetch)
            for tool in fetch gclient; do
                command -v "$tool" >/dev/null 2>&1 || {
                    err "not on PATH: $tool -- clone depot_tools and put it on PATH:"
                    err "  git clone --depth 1 https://chromium.googlesource.com/chromium/tools/depot_tools.git"
                    failures=$((failures + 1))
                }
            done
            ;;
        build)
            for tool in gn ninja lipo xcrun; do
                command -v "$tool" >/dev/null 2>&1 || { err "not on PATH: $tool"; failures=$((failures + 1)); }
            done
            ;;
        xcframework)
            command -v xcodebuild >/dev/null 2>&1 || { err "not on PATH: xcodebuild"; failures=$((failures + 1)); }
            ;;
    esac
    echo "$PINNED_REVISION" | grep -Eq '^[0-9a-f]{40}$' \
        || { err "source.angle_revision is not a 40-character commit hash: $PINNED_REVISION"; failures=$((failures + 1)); }
    return "$failures"
}

if [ "$MODE" = "check" ]; then
    info "pin        $PINNED_REVISION"
    info "repository $ANGLE_REPOSITORY"
    info "source     $SOURCE"
    if check_ready; then ok "ready"; exit 0; fi
    err "not ready"
    exit 1
fi

check_ready || { err "prerequisites unmet; run --check for the list"; exit 1; }

# ---------------------------------------------------------------------------
# Fetch
# ---------------------------------------------------------------------------

if [ "$MODE" = "fetch" ]; then
    mkdir -p "$SOURCE"
    if [ ! -f "$SOURCE/.gclient" ]; then
        info "fetch --no-history angle into $SOURCE (about 12 GB and four minutes)"
        ( cd "$SOURCE" && fetch --no-history angle )
        # target_os limited to the two Apple platforms so gclient does not sync
        # the Android and Windows dependency sets, which are most of the tree
        # and none of the product. Written from the pin so the checkout's shape
        # is part of what the lock file describes.
        python3 - "$SOURCE/.gclient" "$GCLIENT_TARGET_OS" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
platforms = sys.argv[2].split()
with path.open("a", encoding="utf-8") as handle:
    handle.write("target_os = %r\n" % (platforms,))
    handle.write("target_os_only = True\n")
PY
    fi

    ANGLE_ROOT="$(angle_root)"
    info "solution root $ANGLE_ROOT (read from .gclient)"

    # `--revision` and not a bare sync: the whole point of a pin is that the
    # tree is the one at that hash rather than whatever tip happens to be. A
    # `--no-history` checkout is shallow, so reaching an older commit can need
    # the history the fetch deliberately skipped; that is a diagnosable failure
    # with a named recovery rather than a mystery, so it gets one.
    info "gclient sync to $REVISION"
    if ! ( cd "$SOURCE" && gclient sync --no-history --revision "$REVISION" ); then
        err "sync to $REVISION failed against a --no-history checkout; unshallowing and retrying"
        ( cd "$ANGLE_ROOT" && git fetch --unshallow origin ) || true
        ( cd "$SOURCE" && gclient sync --revision "$REVISION" )
    fi

    head="$(cd "$ANGLE_ROOT" && git rev-parse HEAD)"
    if [ "$head" != "$REVISION" ]; then
        err "checkout is at $head, the pin says $REVISION"
        exit 1
    fi
    ok "ANGLE at $REVISION in $ANGLE_ROOT"
    exit 0
fi

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

if [ "$(uname -s)" != "Darwin" ]; then
    err "Building ANGLE for an Apple platform needs macOS: gn takes the SDK from"
    err "xcrun and the Metal backend is compiled by Apple's clang."
    err ""
    err "This host is $(uname -s). Refusing to report a build that did not happen."
    err "What is checkable here:"
    err "  bash scripts/build-angle-apple.sh --print-gn-args aarch64-apple-ios"
    err "  bash scripts/test-apple-angle-recipe-contract.sh"
    exit 1
fi

ANGLE_ROOT="$(angle_root)"
head="$(cd "$ANGLE_ROOT" && git rev-parse HEAD)"
if [ "$head" != "$REVISION" ]; then
    err "the checkout at $ANGLE_ROOT is at $head, not the pinned $REVISION"
    err "run: scripts/build-angle-apple.sh --fetch --source $SOURCE"
    exit 1
fi

if [ "$MODE" = "build" ]; then
    SLICES="$(slices_for_platform "$PLATFORM")" || exit 1
    OUT_DIR="$INSTALL_ROOT/angle-apple-$PLATFORM"

    info "platform  $PLATFORM"
    info "revision  $REVISION"
    info "slices    $(echo $SLICES | tr '\n' ' ')"
    info "targets   $NINJA_TARGETS"

    for triple in $SLICES; do
        args="$(gn_args_for_triple "$triple")" || exit 1
        out="out/migo-$triple"
        info "gn gen $out"
        ( cd "$ANGLE_ROOT" && gn gen "$out" --args="$args" )
        ( cd "$ANGLE_ROOT" && gn args "$out" --list --short --overrides-only )
        info "ninja -C $out $NINJA_TARGETS"
        ( cd "$ANGLE_ROOT" && ninja -C "$out" $NINJA_TARGETS )
    done

    rm -rf "$OUT_DIR"
    mkdir -p "$OUT_DIR"

    for target in $NINJA_TARGETS; do
        product="$target.dylib"
        inputs=""
        for triple in $SLICES; do
            candidate="$ANGLE_ROOT/out/migo-$triple/$product"
            if [ ! -f "$candidate" ]; then
                err "ninja target '$target' produced no $product for $triple"
                err "what the output directory does contain:"
                ls -1 "$ANGLE_ROOT/out/migo-$triple"/*.dylib 2>/dev/null >&2 || err "  (no .dylib at all)"
                exit 1
            fi
            inputs="$inputs $candidate"
        done
        # lipo even for one slice would work, but `cp` keeps a single-slice
        # platform a thin Mach-O rather than a one-architecture fat file --
        # which is what build-apple-sdk.sh produces for the same platform, and
        # matching it keeps the two archives comparable.
        count="$(echo $SLICES | wc -w | tr -d ' ')"
        if [ "$count" -gt 1 ]; then
            info "lipo $count slices into $product"
            lipo -create $inputs -output "$OUT_DIR/$product"
        else
            cp $inputs "$OUT_DIR/$product"
        fi
    done

    # Facts the presenter will need and nobody has: what dyld will call these
    # libraries, what they expect to find beside them, and which architectures
    # actually landed. Printed rather than asserted -- there is no presenter yet
    # to have an opinion, and a log is where the next step reads them from.
    for target in $NINJA_TARGETS; do
        product="$OUT_DIR/$target.dylib"
        info "--- $target.dylib ---"
        ls -l "$product"
        lipo -info "$product" || true
        otool -D "$product" || true
        otool -L "$product" || true
    done

    ok "installed $NINJA_TARGETS into $OUT_DIR"
    exit 0
fi

# ---------------------------------------------------------------------------
# xcframework assembly
# ---------------------------------------------------------------------------

if [ "$MODE" = "xcframework" ]; then
    mkdir -p "$FRAMEWORKS_DIR"
    for target in $NINJA_TARGETS; do
        # `xcodebuild -create-xcframework` holds ONE library per platform, so two
        # dylibs are two xcframeworks. Named ANGLELibEGL / ANGLELibGLESv2
        # because a SwiftPM binaryTarget is named after the xcframework's
        # basename and has to be a valid identifier.
        suffix="$(echo "$target" | sed 's/^lib//')"
        xcframework="$FRAMEWORKS_DIR/ANGLELib$suffix.xcframework"
        libraries=""
        for platform in ios ios-simulator macos; do
            candidate="$INSTALL_ROOT/angle-apple-$platform/$target.dylib"
            [ -f "$candidate" ] || continue
            libraries="$libraries -library $candidate"
        done
        if [ -z "$libraries" ]; then
            err "no installed platform carries $target.dylib; build at least one first:"
            err "  scripts/build-angle-apple.sh --platform ios"
            exit 1
        fi
        rm -rf "$xcframework"
        info "xcodebuild -create-xcframework -> $(basename "$xcframework")"
        xcodebuild -create-xcframework $libraries -output "$xcframework"
    done
    ok "assembled into $FRAMEWORKS_DIR"
    exit 0
fi

err "unreachable mode: $MODE"
exit 1
