#!/usr/bin/env bash
# Build and exercise the Linux Qt 6 Host Kit without building the Migo engine
# or V8. The tests link a strict fake implementation of the public C ABI.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOURCE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
# Staging/debug runs may keep the Host Kit sources outside the repository while
# borrowing its public C headers. Normal repository and CI runs leave this unset.
SDK_SOURCE_ROOT="${MIGO_REPOSITORY_ROOT:-$SOURCE_ROOT}"
HOST_KIT_ROOT="$SOURCE_ROOT/platforms/linux/host-kit"

fail() {
    printf 'Linux Qt Host Kit contract: FAIL: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || fail "missing command '$1'"
}

require_file() {
    [[ -f "$1" ]] || fail "missing file ${1#"$SOURCE_ROOT"/}"
}

require_literal() {
    local path="$1"
    local literal="$2"
    local message="$3"
    rg --fixed-strings --quiet "$literal" "$path" || fail "$message"
}

for command in cmake c++ ninja rg xvfb-run; do
    require_command "$command"
done

require_file "$SDK_SOURCE_ROOT/include/migo/migo.h"
require_file "$HOST_KIT_ROOT/CMakeLists.txt"
require_file "$HOST_KIT_ROOT/README.md"
require_file "$HOST_KIT_ROOT/include/migo/linux/surface_host.hpp"
require_file "$HOST_KIT_ROOT/include/migo/linux/qt6/x11_surface_view.hpp"

if [[ "$SOURCE_ROOT" == "$SDK_SOURCE_ROOT" ]]; then
    for workflow in pr-ci.yml release.yml; do
        require_file "$SOURCE_ROOT/.github/workflows/$workflow"
        require_literal "$SOURCE_ROOT/.github/workflows/$workflow" \
            'bash scripts/test-linux-qt-host-kit.sh' \
            "$workflow must run the Linux Qt Host Kit contract"
    done
fi

PRODUCTION_PATHS=("$HOST_KIT_ROOT/include" "$HOST_KIT_ROOT/src")
if rg -n \
    'Qt[^ >"]+/private/|<private/|qplatformnativeinterface|QQuick|QtQuick|QPainter|glReadPixels|grabFramebuffer|toImage\(|createWindowContainer|fromWinId' \
    "${PRODUCTION_PATHS[@]}"; then
    fail "private Qt API, Qt Quick overlay, or CPU readback escaped into the X11 adapter"
fi
if rg -n 'QMainWindow|QWidget[[:space:]]*\([[:space:]]*nullptr|new[[:space:]]+QWindow|setParent[[:space:]]*\([[:space:]]*nullptr|setWindowFlags?\(|Qt::Window\b' \
    "${PRODUCTION_PATHS[@]}"; then
    fail "Host Kit must not construct a top-level window"
fi

require_literal "$HOST_KIT_ROOT/include/migo/linux/qt6/x11_surface_view.hpp" \
    'MigoQtX11SurfaceView(SurfaceHost &surface_host, QWidget &parent);' \
    "Qt view must require an App-owned parent widget"
require_literal "$HOST_KIT_ROOT/src/qt6/x11_surface_view.cpp" \
    'nativeInterface<QNativeInterface::QX11Application>()' \
    "Qt X11 adapter must use the public Qt native interface"
require_literal "$HOST_KIT_ROOT/src/qt6/x11_surface_view.cpp" \
    '#if !QT_CONFIG(xcb)' \
    "Qt X11 adapter must fail clearly when Qt was built without xcb"
require_literal "$HOST_KIT_ROOT/src/qt6/x11_surface_view.cpp" \
    'Qt::WA_PaintOnScreen' \
    "Qt view must bypass QWidget painting"
require_literal "$HOST_KIT_ROOT/src/qt6/x11_surface_view.cpp" \
    'Qt::WA_DontCreateNativeAncestors' \
    "Qt view must not force the App's widget ancestry native"

BUILD_DIR="$(mktemp -d "${TMPDIR:-/tmp}/migo-linux-host-kit.XXXXXX")"
cleanup() {
    case "$BUILD_DIR" in
        "${TMPDIR:-/tmp}"/migo-linux-host-kit.*) rm -rf -- "$BUILD_DIR" ;;
        *) printf 'Refusing to remove unexpected build directory: %s\n' "$BUILD_DIR" >&2 ;;
    esac
}
trap cleanup EXIT

# The lifecycle controller is a separate product target. Prove its configuration
# does not discover or create Qt targets.
cmake \
    -S "$HOST_KIT_ROOT/tests/controller-only" \
    -B "$BUILD_DIR/controller-only" \
    -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$BUILD_DIR/controller-prefix" \
    -DCMAKE_INSTALL_LIBDIR=lib \
    -DMIGO_LINUX_HOST_KIT_ENABLE_INSTALL=ON \
    -DMIGO_PUBLIC_INCLUDE="$SDK_SOURCE_ROOT/include"
cmake --build "$BUILD_DIR/controller-only" --parallel
cmake --install "$BUILD_DIR/controller-only"
require_file "$BUILD_DIR/controller-prefix/include/migo/linux/surface_host.hpp"
require_file "$BUILD_DIR/controller-prefix/lib/cmake/migo-linux-host-kit/migo-linux-host-kit-config.cmake"
if [[ -e "$BUILD_DIR/controller-prefix/include/migo/linux/qt6/x11_surface_view.hpp" ]]; then
    fail "controller-only install must not publish the disabled Qt adapter header"
fi
if rg --fixed-strings --quiet 'qt6-x11-surface-view' \
    "$BUILD_DIR/controller-prefix/lib/cmake/migo-linux-host-kit/"; then
    fail "controller-only install must not export the disabled Qt adapter target"
fi

cmake \
    -S "$HOST_KIT_ROOT/tests/install-controller-consumer" \
    -B "$BUILD_DIR/install-controller-consumer" \
    -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_PREFIX_PATH="$BUILD_DIR/controller-prefix" \
    -DMIGO_PUBLIC_INCLUDE="$SDK_SOURCE_ROOT/include"
cmake --build "$BUILD_DIR/install-controller-consumer" --parallel
require_file \
    "$BUILD_DIR/install-controller-consumer/migo-linux-surface-host-install-consumer"
"$BUILD_DIR/install-controller-consumer/migo-linux-surface-host-install-consumer"

cmake \
    -S "$HOST_KIT_ROOT/tests" \
    -B "$BUILD_DIR" \
    -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$BUILD_DIR/prefix" \
    -DCMAKE_INSTALL_LIBDIR=lib \
    -DMIGO_LINUX_HOST_KIT_ENABLE_INSTALL=ON \
    -DMIGO_REPOSITORY_ROOT="$SDK_SOURCE_ROOT"
cmake --build "$BUILD_DIR" --parallel
cmake --install "$BUILD_DIR"

require_file "$BUILD_DIR/prefix/include/migo/linux/surface_host.hpp"
require_file "$BUILD_DIR/prefix/include/migo/linux/qt6/x11_surface_view.hpp"
require_file "$BUILD_DIR/prefix/share/doc/migo-linux-host-kit/README.md"
require_file "$BUILD_DIR/prefix/lib/cmake/migo-linux-host-kit/migo-linux-host-kit-config.cmake"
require_file "$BUILD_DIR/prefix/lib/cmake/migo-linux-host-kit/migo-linux-host-kit-targets.cmake"

cmake \
    -S "$HOST_KIT_ROOT/tests/install-consumer" \
    -B "$BUILD_DIR/install-consumer" \
    -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_PREFIX_PATH="$BUILD_DIR/prefix" \
    -DMIGO_PUBLIC_INCLUDE="$SDK_SOURCE_ROOT/include"
cmake --build "$BUILD_DIR/install-consumer" --parallel
require_file "$BUILD_DIR/install-consumer/migo-host-kit-install-consumer"
"$BUILD_DIR/install-consumer/migo-host-kit-install-consumer"

"$BUILD_DIR/migo-surface-host-test"
env QT_QPA_PLATFORM=offscreen "$BUILD_DIR/migo-qt-x11-view-test"
# Pin xcb explicitly: developer desktops often export WAYLAND_DISPLAY, while
# this adapter and its positive-path test intentionally exercise X11 only.
xvfb-run -a env -u WAYLAND_DISPLAY QT_QPA_PLATFORM=xcb \
    "$BUILD_DIR/migo-qt-x11-view-test"

printf 'Linux Qt Host Kit contract: PASS\n'
