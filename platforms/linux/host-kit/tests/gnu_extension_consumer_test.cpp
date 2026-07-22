#include <migo/linux/qt6/x11_surface_view.hpp>

#include <type_traits>

// GCC defines a legacy lowercase `linux` macro in GNU dialects. This target is
// intentionally compiled as gnu++17 so public namespace names cannot collide
// with the default flags used by downstream CMake projects.
static_assert(!std::is_move_constructible_v<migo::linux_host::SurfaceHost>);

void accepts_public_host_kit_types(migo::linux_host::SurfaceHost &host, QWidget &parent) {
    migo::linux_host::qt6::MigoQtX11SurfaceView view(host, parent);
}
