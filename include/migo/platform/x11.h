#ifndef MIGO_PLATFORM_X11_H_
#define MIGO_PLATFORM_X11_H_

#include <migo/surface.h>

/*
 * display is a host-owned Display* and window is its X11 Window/XID. Migo
 * borrows display only for this attach call to identify the X11 server and
 * opens its own render connection. Migo never closes or dispatches events on
 * the host connection and never destroys window. The host must keep window
 * valid until the release observer reaches
 * MIGO_SURFACE_RELEASE_RELEASED.
 */
typedef struct MigoX11WindowDescriptor {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoPlatformKind platform_kind;
    MigoPlatformDescriptorFlags flags;
    void *display;
    uintptr_t window;
    int32_t screen;
    uint32_t reserved0;
} MigoX11WindowDescriptor;

#endif /* MIGO_PLATFORM_X11_H_ */
