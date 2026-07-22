#ifndef MIGO_PLATFORM_WAYLAND_H_
#define MIGO_PLATFORM_WAYLAND_H_

#include <stddef.h> /* offsetof */
#include <migo/surface.h>

/*
 * display and surface are host-owned wl_display* and wl_surface*. The host
 * owns dispatch and the surface role; both objects remain valid until the
 * release observer reaches MIGO_SURFACE_RELEASE_RELEASED.
 */
typedef struct MigoWaylandSurfaceDescriptor {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoPlatformKind platform_kind;
    MigoPlatformDescriptorFlags flags;
    void *display;
    void *surface;
} MigoWaylandSurfaceDescriptor;

MIGO_STATIC_ASSERT(offsetof(MigoWaylandSurfaceDescriptor, struct_size) == 0,
                   "every versioned struct must begin with struct_size");
#if MIGO_LP64
MIGO_STATIC_ASSERT(sizeof(MigoWaylandSurfaceDescriptor) == 32,
                   "MigoWaylandSurfaceDescriptor LP64 size changed");
MIGO_STATIC_ASSERT(offsetof(MigoWaylandSurfaceDescriptor, display) == 16,
                   "MigoWaylandSurfaceDescriptor.display moved");
MIGO_STATIC_ASSERT(offsetof(MigoWaylandSurfaceDescriptor, surface) == 24,
                   "MigoWaylandSurfaceDescriptor.surface moved");
#endif

#endif /* MIGO_PLATFORM_WAYLAND_H_ */
