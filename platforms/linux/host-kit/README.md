# Migo Linux Host Kit

This directory contains source-level adapters for embedding Migo in an
application-owned Linux UI. It does not create an application, a top-level
window, or an event loop.

The current milestone is deliberately narrow:

- `migo::linux-surface-host` is a toolkit-neutral C++17 lifecycle controller
  for X11 and Wayland handles supplied by a host.
- `migo::qt6-x11-surface-view` is a Qt 6.4+ Widgets adapter for the `xcb`
  platform plugin. It is surface-only and uses a native child `QWidget`.
- Qt Wayland, Qt Quick, GTK convenience widgets, input translation and a
  Managed Session wrapper are not claimed by this milestone.

The public Migo C ABI is still marked `MIGO_C_ABI_CANDIDATE`. Treat this Host
Kit as an integration preview until that ABI is frozen.

## Ownership contract

The application owns the `MigoSession`, `SurfaceHost`, parent widget and native
window. Keep both dependency chains valid:

```text
QApplication > parent QWidget > MigoQtX11SurfaceView
MigoSession > SurfaceHost ───────> MigoQtX11SurfaceView
```

Create exactly one `SurfaceHost` for a Session and keep it at a stable address
for that Session's surface lifetime. Replacement views borrow the same
controller so surface generations stay strictly increasing. A Session has at
most one Attached or Retiring view; attach a replacement only after the prior
release reaches RELEASED. A replacement may be constructed earlier, but it
remains passive: its resize, close and destruction cannot act on the current
owner's generation. The controller
never owns the Session, X11 `Display`, XID, `wl_display`, or `wl_surface`.
`QApplication` must already exist before the Qt view and must outlive its final
release, because Qt owns the X11 connection and GUI event loop. The
toolkit-neutral `SurfaceHost` itself has no such Qt dependency.
After a view successfully attaches, use that view's resize/detach/poll path for
its generation; calling the borrowed controller directly would bypass the
native-window lifetime guards.

Before deleting the view or any ancestor that owns its native child window,
call `close()` or `beginDetach()`, then keep the widget and GUI event loop alive
until `surfaceReleased` is emitted. A pending release is a real driver lease;
destroying the window early is a use-after-free. The adapter fails fast if Qt
tries to destroy the native surface before release instead of hiding this bug.
Polling is fast only during the expected short retirement window, then backs
off; after two seconds `surfaceReleaseStalled` fires once for diagnostics while
the authoritative observer remains alive and continues to be checked.
If the ABI query/destroy operation itself returns an error, automatic polling
pauses to avoid an error storm but the view stays Retiring and keeps ownership;
the host must diagnose the error and explicitly call `pollDetach()` to retry.
Reparenting may recreate the XID and therefore is allowed only after the view is
Detached; attach it again after the new parent/native child is established.
`hide()` alone does not retire the native target or change Session lifecycle;
the Bound host decides whether a hidden pane stays warm, pauses the Session, or
calls `beginDetach()` to release presentation resources.

Construct `SurfaceHost` on the Qt GUI thread that owns the Session. All methods
must stay on that thread and be serialized with other calls through the same
Session; foreign-thread calls return `MIGO_ERROR_WRONG_THREAD` before entering
the C ABI. The QWidget adapter also rejects a foreign thread before reading or
mutating any Qt state. The Bound Session host remains responsible for
callbacks, lifecycle/focus, content loading, input and audio-focus policy. Migo
continues to own its default audio renderer; this surface-only adapter neither
opens an audio device nor provides a host-mixer API.

## Build and consume from source

On Ubuntu 24.04, the exact dependency set for the current Qt Widgets/X11
adapter and its contract tests is:

```bash
sudo apt-get update
sudo apt-get install -y \
  cmake ninja-build g++ ripgrep xvfb xauth \
  qt6-base-dev qt6-base-dev-tools qt6-qpa-plugins \
  libx11-dev libxkbcommon-x11-dev libxcb-cursor0
```

The adapter's compile-time API uses Qt Core/Gui/Widgets and Xlib. The remaining
packages supply the xcb platform plugin's runtime dependencies and the isolated
Xvfb positive-path test. A native Wayland host that consumes only the
toolkit-neutral controller also needs its distribution's Wayland development
package (on Ubuntu: `libwayland-dev`), but that does not turn the Qt adapter
into a Wayland implementation. Do not install Qt private-header packages for
this Host Kit.

The effective deployment floor is the stricter of the core SDK manifest
(`glibc`, GLIBCXX, arch and CPU baseline) and the Qt distribution selected by
the application. “Qt 6.4+” is an adapter API floor, not a claim that every Qt
6.4 package runs on every glibc 2.31 distribution. Official prebuilt Host Kit
artifacts, if introduced later, must record both identities in their own
artifact manifest plus the exact compatible core SDK manifest hash (which pins
V8 revision, GN arguments and snapshot policy/inputs). The current source build
inherits the application's toolchain and Qt ABI instead.

After installing or staging the Migo Linux SDK so `find_package(migo)` works:

```cmake
find_package(migo CONFIG REQUIRED)
add_subdirectory(path/to/migo/platforms/linux/host-kit migo-host-kit)

target_link_libraries(my_app PRIVATE migo::qt6-x11-surface-view)
```

Applications that provide their own native integration can link only
`migo::linux-surface-host`; this keeps Qt out of their dependency graph. The
Host Kit can also be installed and consumed. Install rules default to ON
for a standalone Host Kit configure and OFF when it is an App subdirectory, so
embedding it cannot silently change the App's install manifest:

```bash
cmake -S platforms/linux/host-kit -B build/host-kit \
  -DCMAKE_PREFIX_PATH=/path/to/migo-sdk \
  -DCMAKE_INSTALL_PREFIX=/path/to/prefix \
  -DMIGO_LINUX_HOST_KIT_ENABLE_INSTALL=ON
cmake --build build/host-kit
cmake --install build/host-kit
```

The installed consumer then uses:

```cmake
find_package(migo-linux-host-kit CONFIG REQUIRED)
target_link_libraries(my_app PRIVATE migo::qt6-x11-surface-view)
```

Minimal construction after the application has created a Session:

```cpp
class GamePane final : public QWidget {
public:
    GamePane(MigoSession *session, QWidget &parent)
        : QWidget(&parent), surface_host_(session), game_view_(surface_host_, *this) {
        layout_.addWidget(&game_view_);
        setLayout(&layout_);
    }

private:
    QVBoxLayout layout_;
    migo::linux_host::SurfaceHost surface_host_;
    migo::linux_host::qt6::MigoQtX11SurfaceView game_view_;
};
```

Production shutdown must wait for `surfaceReleased` before destroying
`GamePane`. A real application should connect that signal to its asynchronous
window/session shutdown coordinator rather than blocking the GUI thread.

## Render-path limits

The X11 widget is a direct native-surface path: Migo presents to the child XID
without CPU copies, `QPainter`, framebuffer readback or an intermediate Qt
texture. It keeps the host's ancestor widgets non-native, so embedding one game
surface does not impose native-window painting and resize costs on the whole UI
tree. Qt Quick uses a compositor-owned scene graph, so a child-window
overlay would break clipping, transforms and frame scheduling. It remains
unsupported until Migo exposes an explicit zero-copy texture plus synchronization
contract. Qt 6.4 also does not expose the Wayland display/surface pair needed by
this adapter through a supported public API; private Qt headers are forbidden.

Run the isolated contract without building V8:

```bash
bash scripts/test-linux-qt-host-kit.sh
```
