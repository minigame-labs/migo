#ifndef MIGO_PLATFORM_WAYLAND_H_
#define MIGO_PLATFORM_WAYLAND_H_

#include <migo/surface.h>

/*
 * display and surface are host-owned wl_display* and wl_surface*. The host
 * owns dispatch and the surface role; both objects remain valid until detach.
 */
typedef struct MigoWaylandSurfaceDescriptor {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoPlatformKind platform_kind;
    MigoPlatformDescriptorFlags flags;
    void *display;
    void *surface;
} MigoWaylandSurfaceDescriptor;

#endif /* MIGO_PLATFORM_WAYLAND_H_ */
