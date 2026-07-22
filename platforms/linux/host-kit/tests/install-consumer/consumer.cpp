#include <migo/linux/qt6/x11_surface_view.hpp>

#include <type_traits>

static_assert(!std::is_move_constructible_v<migo::linux_host::SurfaceHost>);

void accepts_installed_types(migo::linux_host::SurfaceHost &host, QWidget &parent) {
    migo::linux_host::qt6::MigoQtX11SurfaceView view(host, parent);
}

int main() {
    // This binary is a link-closure test, not a GUI smoke test. The external
    // function above keeps the Qt adapter constructor/destructor referenced;
    // executing it would require a QApplication and an active display.
    return 0;
}
