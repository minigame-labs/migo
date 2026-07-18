#ifndef MIGO_PLATFORM_ANDROID_H_
#define MIGO_PLATFORM_ANDROID_H_

#include <migo/surface.h>

/*
 * native_window is an ANativeWindow*. A future implementation acquires its
 * own strong reference before attach returns and releases it during detach.
 */
typedef struct MigoAndroidNativeWindowDescriptor {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoPlatformKind platform_kind;
    MigoPlatformDescriptorFlags flags;
    void *native_window;
} MigoAndroidNativeWindowDescriptor;

#endif /* MIGO_PLATFORM_ANDROID_H_ */
