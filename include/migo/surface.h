#ifndef MIGO_SURFACE_H_
#define MIGO_SURFACE_H_

#include <migo/types.h>

typedef uint32_t MigoPlatformKind;
#define MIGO_PLATFORM_UNKNOWN UINT32_C(0)
#define MIGO_PLATFORM_ANDROID_NATIVE_WINDOW UINT32_C(1)
#define MIGO_PLATFORM_WIN32_HWND UINT32_C(2)
#define MIGO_PLATFORM_WINUI_SWAP_CHAIN_PANEL UINT32_C(3)
#define MIGO_PLATFORM_MACOS_NS_VIEW UINT32_C(4)
#define MIGO_PLATFORM_MACOS_CA_METAL_LAYER UINT32_C(5)
#define MIGO_PLATFORM_X11_WINDOW UINT32_C(6)
#define MIGO_PLATFORM_WAYLAND_SURFACE UINT32_C(7)
#define MIGO_PLATFORM_OPENHARMONY_NATIVE_WINDOW UINT32_C(8)

typedef uint32_t MigoSurfaceDescriptorFlags;
#define MIGO_SURFACE_DESCRIPTOR_FLAG_NONE UINT32_C(0)

typedef uint32_t MigoPlatformDescriptorFlags;
#define MIGO_PLATFORM_DESCRIPTOR_FLAG_NONE UINT32_C(0)

typedef uint32_t MigoColorSpace;
#define MIGO_COLOR_SPACE_UNSPECIFIED UINT32_C(0)
#define MIGO_COLOR_SPACE_SRGB UINT32_C(1)
#define MIGO_COLOR_SPACE_DISPLAY_P3 UINT32_C(2)
#define MIGO_COLOR_SPACE_EXTENDED_SRGB UINT32_C(3)

typedef uint32_t MigoAlphaMode;
#define MIGO_ALPHA_MODE_UNSPECIFIED UINT32_C(0)
#define MIGO_ALPHA_MODE_OPAQUE UINT32_C(1)
#define MIGO_ALPHA_MODE_PREMULTIPLIED UINT32_C(2)
#define MIGO_ALPHA_MODE_POSTMULTIPLIED UINT32_C(3)

typedef uint32_t MigoPresentationMode;
#define MIGO_PRESENTATION_MODE_DEFAULT UINT32_C(0)
#define MIGO_PRESENTATION_MODE_FIFO UINT32_C(1)
#define MIGO_PRESENTATION_MODE_MAILBOX UINT32_C(2)
#define MIGO_PRESENTATION_MODE_IMMEDIATE UINT32_C(3)

typedef uint64_t MigoSurfaceCapabilities;
#define MIGO_SURFACE_CAPABILITY_NONE UINT64_C(0)
#define MIGO_SURFACE_CAPABILITY_WIDE_COLOR (UINT64_C(1) << 0)
#define MIGO_SURFACE_CAPABILITY_TRANSPARENT (UINT64_C(1) << 1)
#define MIGO_SURFACE_CAPABILITY_MAILBOX_PRESENT (UINT64_C(1) << 2)

typedef uint32_t MigoSurfaceLossReason;
#define MIGO_SURFACE_LOSS_UNKNOWN UINT32_C(0)
#define MIGO_SURFACE_LOSS_HOST_DESTROYED UINT32_C(1)
#define MIGO_SURFACE_LOSS_DEVICE_LOST UINT32_C(2)
#define MIGO_SURFACE_LOSS_PLATFORM_ERROR UINT32_C(3)

/* Common prefix repeated by every strongly typed platform descriptor. */
typedef struct MigoPlatformSurfaceDescriptor {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoPlatformKind platform_kind;
    MigoPlatformDescriptorFlags flags;
} MigoPlatformSurfaceDescriptor;

typedef struct MigoSurfaceMetrics {
    uint32_t struct_size;
    uint32_t abi_version;
    uint64_t generation;
    uint32_t width_pixels;
    uint32_t height_pixels;
    float scale_factor;
    MigoColorSpace color_space;
    MigoAlphaMode alpha_mode;
    MigoPresentationMode preferred_presentation_mode;
    MigoSurfaceDescriptorFlags flags;
    uint32_t reserved0;
} MigoSurfaceMetrics;

/*
 * platform_descriptor is borrowed for this call only. The implementation must
 * copy the descriptor and acquire any ref-counted native target before attach
 * returns success. platform_descriptor_size must equal the typed descriptor's
 * struct_size for ABI v1; the duplicate value is an intentional envelope versus
 * payload bounds/integrity cross-check. Every reserved field supplied by the
 * caller must be zero.
 */
typedef struct MigoSurfaceDescriptor {
    uint32_t struct_size;
    uint32_t abi_version;
    uint64_t generation;
    MigoPlatformKind platform_kind;
    MigoSurfaceDescriptorFlags flags;
    uint32_t width_pixels;
    uint32_t height_pixels;
    float scale_factor;
    MigoColorSpace color_space;
    MigoAlphaMode alpha_mode;
    MigoPresentationMode preferred_presentation_mode;
    MigoSurfaceCapabilities capability_flags;
    uint32_t platform_descriptor_size;
    uint32_t reserved0;
    const void *platform_descriptor;
} MigoSurfaceDescriptor;

MIGO_BEGIN_DECLS

MIGO_API MigoResult MIGO_CALL migo_session_attach_surface(
    MigoSession *session,
    const MigoSurfaceDescriptor *descriptor,
    MigoSurfaceAttachment **out_attachment);

MIGO_API MigoResult MIGO_CALL migo_surface_update(
    MigoSurfaceAttachment *attachment,
    const MigoSurfaceMetrics *metrics);

/*
 * Synchronous completion boundary. attachment is a unique handle and must not
 * be copied into independently owned aliases. MIGO_OK consumes and releases
 * the handle: after return, the pointer is invalid and no future GPU operation
 * or presentation may reference its generation. Calling any API again through
 * that pointer is invalid. A non-OK result leaves ownership with the caller so
 * detach can be retried or the owning Session can be destroyed. Detach must not
 * enqueue or wait for another task through the host dispatcher. If required
 * platform-thread affinity is not satisfied, it returns MIGO_ERROR_WRONG_THREAD
 * before retiring the generation or changing ownership.
 */
MIGO_API MigoResult MIGO_CALL
migo_surface_detach(MigoSurfaceAttachment *attachment);

MIGO_END_DECLS

#endif /* MIGO_SURFACE_H_ */
