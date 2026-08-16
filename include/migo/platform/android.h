#ifndef MIGO_PLATFORM_ANDROID_H_
#define MIGO_PLATFORM_ANDROID_H_

#include <migo/surface.h>

/*
 * native_window is an ANativeWindow*. Migo acquires its own strong reference
 * before attach returns success and releases that reference before the release
 * observer reaches MIGO_SURFACE_RELEASE_RELEASED.
 */
typedef struct MigoAndroidNativeWindowDescriptor {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoPlatformKind platform_kind;
    MigoPlatformDescriptorFlags flags;
    void *native_window;
} MigoAndroidNativeWindowDescriptor;

/*
 * Registers this process's JavaVM and an Activity/Context jobject, which the
 * Android audio backend needs before it can open an output stream. The
 * Java-based platform library does this for itself; a pure C host has no JNI
 * of its own and must call this explicitly, once, before creating any session
 * that will play audio -- otherwise the first audio playback aborts the
 * process instead of failing an op.
 *
 * vm and activity are the same pointers an ANativeActivity already carries as
 * its `vm` and `clazz` fields. activity may be NULL for an audio-only
 * embedding. Calls after the first are a no-op, so a host may call this again
 * after an activity is recreated without checking whether it already has.
 *
 * Returns MIGO_ERROR_INVALID_ARGUMENT if vm is NULL.
 */
MIGO_API MigoResult MIGO_CALL migo_android_init_context(void *vm, void *activity);

#endif /* MIGO_PLATFORM_ANDROID_H_ */
