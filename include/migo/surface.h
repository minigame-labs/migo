#ifndef MIGO_SURFACE_H_
#define MIGO_SURFACE_H_

#include <stddef.h> /* offsetof */
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

MIGO_STATIC_ASSERT(sizeof(MigoSurfaceMetrics) == 48, "MigoSurfaceMetrics size changed");
MIGO_STATIC_ASSERT(offsetof(MigoSurfaceMetrics, struct_size) == 0,
                   "every versioned struct must begin with struct_size");
MIGO_STATIC_ASSERT(offsetof(MigoSurfaceMetrics, scale_factor) == 24, "MigoSurfaceMetrics.scale_factor moved");

/*
 * platform_descriptor is borrowed for this call only. The implementation must
 * copy the descriptor and acquire any ref-counted native target before attach
 * returns success. platform_descriptor_size must equal the typed descriptor's
 * struct_size for ABI v1; the duplicate value is an intentional envelope versus
 * payload bounds/integrity cross-check. Every reserved field supplied by the
 * caller must be zero.
 *
 * generation numbers the host's attachments and starts at 1. Each attach must
 * carry one strictly greater than any this Session has already accepted -- that
 * is what lets the engine discard work naming a window the host has since
 * replaced -- and a repeated value is refused with MIGO_ERROR_STALE_SURFACE. A
 * refused attach consumes nothing, so a retry may offer the same value again.
 * Any platform that destroys and recreates the window during normal operation,
 * which Android does on every trip through the background, therefore needs a
 * counter rather than a constant. MigoSurfaceMetrics.generation is the mirror
 * rule: an update names the attachment it updates, so it carries that
 * attachment's own generation rather than the next one.
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

MIGO_STATIC_ASSERT(offsetof(MigoSurfaceDescriptor, struct_size) == 0,
                   "every versioned struct must begin with struct_size");
#if MIGO_LP64
MIGO_STATIC_ASSERT(sizeof(MigoSurfaceDescriptor) == 72, "MigoSurfaceDescriptor LP64 size changed");
MIGO_STATIC_ASSERT(offsetof(MigoSurfaceDescriptor, capability_flags) == 48,
                   "MigoSurfaceDescriptor.capability_flags moved");
MIGO_STATIC_ASSERT(offsetof(MigoSurfaceDescriptor, platform_descriptor) == 64,
                   "MigoSurfaceDescriptor.platform_descriptor moved");
#endif

/*
 * Opaque observer of one asynchronous native-Surface release.
 *
 * It owns no Surface resource lease. Once it reports RELEASED it may outlive a
 * later successful destruction of the Session that produced it. A pending
 * observer cannot: migo_session_destroy refuses while any release is pending.
 */
typedef struct MigoSurfaceRelease MigoSurfaceRelease;

/* Level-triggered, not edge-triggered: a late first query still observes a
 * release that already completed. An edge would be missable exactly once, on
 * the path where missing it means destroying a window the GPU still reads. */
typedef uint32_t MigoSurfaceReleaseState;
#define MIGO_SURFACE_RELEASE_PENDING UINT32_C(0)
#define MIGO_SURFACE_RELEASE_RELEASED UINT32_C(1)

typedef struct MigoSurfaceReleaseStatus {
    uint32_t struct_size;
    uint32_t abi_version;
    uint64_t generation;
    MigoSurfaceReleaseState state;
    uint32_t reserved0;
} MigoSurfaceReleaseStatus;

MIGO_STATIC_ASSERT(sizeof(MigoSurfaceReleaseStatus) == 24,
                   "MigoSurfaceReleaseStatus size changed");
MIGO_STATIC_ASSERT(offsetof(MigoSurfaceReleaseStatus, struct_size) == 0,
                   "every versioned struct must begin with struct_size");
MIGO_STATIC_ASSERT(offsetof(MigoSurfaceReleaseStatus, generation) == 8,
                   "MigoSurfaceReleaseStatus.generation moved");
MIGO_STATIC_ASSERT(offsetof(MigoSurfaceReleaseStatus, state) == 16,
                   "MigoSurfaceReleaseStatus.state moved");

MIGO_BEGIN_DECLS

/*
 * The first successful attach fixes the Session's graphics platform identity.
 * Later native-target replacement is supported only within that identity:
 * Android in the same process, X11 on the same server using the Session's
 * private render connection, Wayland on the same wl_display, or HWND under the
 * same ANGLE device. A different
 * backend/display returns MIGO_ERROR_INVALID_STATE synchronously, publishes no
 * attachment, and enqueues no render command; the Session remains retryable.
 */
MIGO_API MigoResult MIGO_CALL migo_session_attach_surface(
    MigoSession *session,
    const MigoSurfaceDescriptor *descriptor,
    MigoSurfaceAttachment **out_attachment);

/*
 * Report a resize or a presentation-parameter change on an already-attached
 * Surface. Synchronous: by the time this returns, either the new metrics are
 * committed and a resize command is enqueued to the host, or nothing changed.
 *
 * metrics.generation must be a monotonically increasing sequence the caller
 * assigns per update; it lets a host that coalesces resize events on its own
 * thread detect and discard an update that a newer one has already
 * superseded.
 *
 *   MIGO_ERROR_INVALID_ARGUMENT  attachment or metrics was NULL; metrics'
 *                                struct_size is smaller than the minimum
 *                                versioned record; a field is out of range
 *                                (zero width/height, non-finite scale
 *                                factor); or metrics.generation is zero or
 *                                newer than the attachment has ever seen
 *   MIGO_ERROR_UNSUPPORTED_ABI   metrics.abi_version does not match this
 *                                engine build, or struct_size claims a
 *                                record larger than this build knows
 *   MIGO_ERROR_INVALID_STATE     another Surface transition (attach, update,
 *                                or detach) is already running on this
 *                                Session, or the Session has no live host
 *   MIGO_ERROR_STALE_SURFACE     attachment is not the Session's active
 *                                attachment, has already been lost, or
 *                                metrics.generation is older than an update
 *                                already applied
 *   MIGO_ERROR_INTERNAL          the host-side lease or dispatch failed;
 *                                rare, and logged when it happens
 *
 * Callable from the thread that owns the Session; concurrent calls through
 * the same Session are the caller's to serialize.
 */
MIGO_API MigoResult MIGO_CALL migo_surface_update(
    MigoSurfaceAttachment *attachment,
    const MigoSurfaceMetrics *metrics);

/*
 * Begin retiring one attachment. This is the irreversible presentation
 * boundary, and it is asynchronous because the GPU cannot be made to forget a
 * Surface synchronously: driver-side references outlive the call.
 *
 * attachment is a unique handle and must not be copied into independently owned
 * aliases. MIGO_OK consumes it -- the pointer is invalid on return, no future
 * GPU operation or presentation references its generation, and *out_release
 * owns a new observer the caller must eventually destroy. out_release is set to
 * NULL before any fallible work, so it is always well-defined to read.
 *
 * A non-OK result changes nothing and consumes nothing:
 *   MIGO_ERROR_INVALID_ARGUMENT  attachment or out_release was NULL
 *   MIGO_ERROR_INVALID_STATE     another Surface transition is already running,
 *                                or the Session has no live host
 *   MIGO_ERROR_STALE_SURFACE     attachment is not the Session's active one
 *
 * This call must not wait for another turn of the host dispatcher. It is
 * callable from the thread that owns the Session; concurrent calls through the
 * same Session are the caller's to serialize.
 *
 * THE HOST MUST NOT DESTROY THE NATIVE WINDOW HERE. Returning MIGO_OK means
 * retirement started, not that the driver is finished. Keep the native
 * resource and its event loop alive until migo_surface_release_query reports
 * MIGO_SURFACE_RELEASE_RELEASED; destroying it earlier is a use-after-free
 * inside the driver, which the engine cannot detect or prevent.
 */
MIGO_API MigoResult MIGO_CALL migo_surface_begin_detach(
    MigoSurfaceAttachment *attachment,
    MigoSurfaceRelease **out_release);

/*
 * Read the authoritative release state. Never blocks, so it is safe to poll
 * from a UI thread or an event-loop idle handler.
 *
 * A release that has reached RELEASED stays valid and queryable after the
 * owning Session is destroyed. Session destruction refuses while it is still
 * PENDING. Returns
 * MIGO_ERROR_INVALID_ARGUMENT if either pointer is NULL. out_status is written
 * only on MIGO_OK, never partially.
 */
MIGO_API MigoResult MIGO_CALL migo_surface_release_query(
    const MigoSurfaceRelease *release,
    MigoSurfaceReleaseStatus *out_status);

/*
 * Destroy a completed observer. MIGO_OK consumes the handle and its pointer
 * becomes invalid.
 *
 * Returns MIGO_ERROR_INVALID_STATE while the release is still
 * MIGO_SURFACE_RELEASE_PENDING, leaving ownership with the caller. Refusing is
 * the point: destroying the observer early would discard the only means of
 * learning when the native resource became safe to free, and the host would
 * have nothing left to wait on.
 */
MIGO_API MigoResult MIGO_CALL
migo_surface_release_destroy(MigoSurfaceRelease *release);

MIGO_END_DECLS

#endif /* MIGO_SURFACE_H_ */
