#include <migo/linux/surface_host.hpp>

#include <type_traits>

static_assert(!std::is_move_constructible_v<migo::linux_host::SurfaceHost>);

std::uint64_t current_generation(const migo::linux_host::SurfaceHost &host) {
    return host.generation();
}
