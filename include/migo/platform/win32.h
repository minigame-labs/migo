#ifndef MIGO_PLATFORM_WIN32_H_
#define MIGO_PLATFORM_WIN32_H_

#include <migo/surface.h>

/*
 * hwnd is a host-owned child HWND. Migo never destroys it or owns the host
 * message loop. The HWND must remain valid until the release observer reaches
 * MIGO_SURFACE_RELEASE_RELEASED.
 */
typedef struct MigoWin32HwndDescriptor {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoPlatformKind platform_kind;
    MigoPlatformDescriptorFlags flags;
    void *hwnd;
} MigoWin32HwndDescriptor;

#endif /* MIGO_PLATFORM_WIN32_H_ */
