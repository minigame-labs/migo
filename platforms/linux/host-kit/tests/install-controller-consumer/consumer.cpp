#include <migo/linux/surface_host.hpp>

#include <type_traits>

static_assert(!std::is_move_constructible_v<migo::linux_host::SurfaceHost>);

std::uint64_t installed_generation(const migo::linux_host::SurfaceHost &host) {
    return host.generation();
}

int main() {
    migo::linux_host::SurfaceHost host(nullptr);
    return installed_generation(host) == 0 ? 0 : 1;
}
