#ifndef MIGO_PLATFORM_OPENHARMONY_H_
#define MIGO_PLATFORM_OPENHARMONY_H_

#include <migo/surface.h>

/*
 * native_window is an OHNativeWindow*. A future implementation takes its own
 * native-object reference before attach returns and releases it during detach.
 */
typedef struct MigoOpenHarmonyNativeWindowDescriptor {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoPlatformKind platform_kind;
    MigoPlatformDescriptorFlags flags;
    void *native_window;
} MigoOpenHarmonyNativeWindowDescriptor;

#endif /* MIGO_PLATFORM_OPENHARMONY_H_ */
