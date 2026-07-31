#ifndef MIGO_TYPES_H_
#define MIGO_TYPES_H_

#include <stddef.h>
#include <stdint.h>

#if defined(_WIN32) || defined(__CYGWIN__)
#define MIGO_CALL __cdecl
#if defined(MIGO_BUILD_SHARED)
#define MIGO_API __declspec(dllexport)
#elif defined(MIGO_USE_SHARED)
#define MIGO_API __declspec(dllimport)
#else
#define MIGO_API
#endif
#elif defined(__GNUC__) || defined(__clang__)
#define MIGO_CALL
#define MIGO_API __attribute__((visibility("default")))
#else
#define MIGO_CALL
#define MIGO_API
#endif

#ifdef __cplusplus
#define MIGO_BEGIN_DECLS extern "C" {
#define MIGO_END_DECLS }
#else
#define MIGO_BEGIN_DECLS
#define MIGO_END_DECLS
#endif

/*
 * This source-visible ABI is still a design candidate. The remaining freeze
 * blockers and target verification matrix are listed in README.md. Do not
 * treat it as stable merely because linkable Android and Linux packages exist.
 */
#define MIGO_C_ABI_CANDIDATE 1

/*
 * Whether a linkable runtime implementing these declarations exists for the
 * platform being compiled for.
 *
 * Desktop Linux ships one: scripts/build-linux-sdk.sh produces libmigo.so and
 * libmigo.a exporting exactly the migo_* set declared here, with pkg-config and
 * CMake integration.
 *
 * Android ships one too, as a static library: scripts/build-android-c-host.sh
 * cross-compiles it, and tests/c_host/android is a NativeActivity with no
 * Java of its own that links it and drives surface attach, lifecycle, touch,
 * the soft keyboard, physical keys, IME composition and gamepads on device.
 * The separate libmigo.so that the Java/JNI SDK ships still exports no migo_*
 * symbols -- that is a different artifact, and this macro is about whether a
 * linkable runtime exists, not about which artifact carries it.
 *
 * The Android C host SDK packages that static library with headers and CMake
 * metadata. It deliberately has no pkg-config file or versioned shared object:
 * an NDK consumer links the archive into its own native shared library.
 *
 * This says a runtime exists, not that the ABI is frozen; MIGO_C_ABI_CANDIDATE
 * remains 1 until the README's blockers are closed.
 *
 * It also describes the headers you compiled against, not the library you
 * linked -- it cannot do otherwise, being a preprocessor macro. Ask
 * migo_query_capabilities (capabilities.h) about the library itself.
 */
/*
 * These three targets must be told apart before the question can be answered.
 * Android and OpenHarmony both define __linux__ -- their kernels are Linux --
 * so testing __linux__ alone silently answers for three different ABIs at
 * once, across a glibc, a Bionic and a musl userspace.
 * __ANDROID__ comes from the NDK; OpenHarmony's toolchain defines __OHOS__.
 * Order matters: the specific targets must be excluded before the generic one.
 */
#if defined(__ANDROID__)
#define MIGO_PLATFORM_IS_ANDROID 1
#else
#define MIGO_PLATFORM_IS_ANDROID 0
#endif

#if defined(__OHOS__) || defined(__OHOS_FAMILY__)
#define MIGO_PLATFORM_IS_OPENHARMONY 1
#else
#define MIGO_PLATFORM_IS_OPENHARMONY 0
#endif

/* Desktop Linux with glibc: what build-linux-sdk.sh targets. Deliberately not
 * "any non-Android Linux" -- a musl or Bionic userspace is a different ABI with
 * a different floor, and claiming this one for it would be a false promise. */
#if defined(__linux__) && !MIGO_PLATFORM_IS_ANDROID && !MIGO_PLATFORM_IS_OPENHARMONY \
    && defined(__GLIBC__)
#define MIGO_PLATFORM_IS_LINUX_GNU 1
#else
#define MIGO_PLATFORM_IS_LINUX_GNU 0
#endif

/*
 * Whether this platform has a linkable migo runtime.
 *
 * A third party greps this to decide whether the platform is usable at all, so
 * the bar for setting it is not "does something link" but "can a surface be
 * attached and driven" -- the bar Android was held to, and the reason
 * OpenHarmony stayed 0 for a while after its static library already linked.
 *
 * OpenHarmony cleared that bar on 2026-07-31 on an API 20 emulator: an
 * XComponent's OHNativeWindow* attached (generation 1), content loaded and
 * reported ready, the surface rendered, and a full touch lifecycle reached JS
 * and was confirmed by reading the rendered pixel back -- red before the tap,
 * blue after the finger lifted. scripts/build-ohos-sdk.sh stages the package
 * for aarch64 and x86_64 and scripts/test-ohos-sdk-contract.sh gates it,
 * including that the manifest claims no platform kind the library cannot
 * actually attach.
 *
 * The macro still cannot describe the library that was linked -- only the
 * platform it was compiled for. migo_query_capabilities answers the narrower
 * question and should be preferred wherever the answer must be exact.
 */
#if MIGO_PLATFORM_IS_LINUX_GNU || MIGO_PLATFORM_IS_ANDROID || MIGO_PLATFORM_IS_OPENHARMONY
#define MIGO_C_ABI_HAS_RUNTIME 1
#else
#define MIGO_C_ABI_HAS_RUNTIME 0
#endif

/*
 * Layout assertions, in the headers a host actually compiles.
 *
 * The Rust implementation pins the same numbers. Two independent assertions of
 * one shape is the point: a change on either side fails on that side, at the
 * moment it is made, instead of surfacing as a host writing one field and the
 * library reading another.
 */
#if defined(__cplusplus)
#define MIGO_STATIC_ASSERT(cond, msg) static_assert(cond, msg)
#else
#define MIGO_STATIC_ASSERT(cond, msg) _Static_assert(cond, msg)
#endif

/* Shipping runtime slices are currently 64-bit, but the public ABI is checked
 * on LP64, Windows LLP64, and ILP32 with both C and C++ compilers. This macro is
 * a layout helper only; it must never be used as a support-target decision. */
#if UINTPTR_MAX == UINT64_MAX
#define MIGO_LP64 1
#else
#define MIGO_LP64 0
#endif

#define MIGO_ABI_VERSION_1 UINT32_C(1)
#define MIGO_ABI_VERSION_CURRENT MIGO_ABI_VERSION_1

typedef int32_t MigoResult;

#define MIGO_OK ((MigoResult)0)
#define MIGO_ERROR_INVALID_ARGUMENT ((MigoResult)-1)
#define MIGO_ERROR_UNSUPPORTED_ABI ((MigoResult)-2)
#define MIGO_ERROR_UNSUPPORTED_PLATFORM ((MigoResult)-3)
#define MIGO_ERROR_UNSUPPORTED_CAPABILITY ((MigoResult)-4)
#define MIGO_ERROR_INVALID_STATE ((MigoResult)-5)
#define MIGO_ERROR_WRONG_THREAD ((MigoResult)-6)
#define MIGO_ERROR_STALE_SURFACE ((MigoResult)-7)
#define MIGO_ERROR_CANCELLED ((MigoResult)-8)
#define MIGO_ERROR_DISPATCH_REJECTED ((MigoResult)-9)
#define MIGO_ERROR_OUT_OF_MEMORY ((MigoResult)-10)
#define MIGO_ERROR_INTERNAL ((MigoResult)-11)

/*
 * The host command queue was full, so the event was not delivered. Unlike every
 * other error here this one is transient: the same call may succeed later. It
 * exists because dropping input silently is worse than reporting it -- a lost
 * MIGO_TOUCH_END leaves content believing a finger is still down, and no later
 * event corrects that. The host decides whether to retry or coalesce, because
 * only the host knows which of its events are safe to merge.
 */
#define MIGO_ERROR_WOULD_BLOCK ((MigoResult)-12)

typedef uint32_t MigoErrorFlags;
#define MIGO_ERROR_FLAG_NONE UINT32_C(0)
#define MIGO_ERROR_FLAG_RECOVERABLE (UINT32_C(1) << 0)

typedef struct MigoEngine MigoEngine;
typedef struct MigoSession MigoSession;
typedef struct MigoSurfaceAttachment MigoSurfaceAttachment;

/*
 * message_utf8 is not NUL-termination-dependent. It is borrowed only for the
 * duration of the callback that received this record. Implementations set
 * reserved0 to zero.
 */
typedef struct MigoError {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoResult code;
    MigoErrorFlags flags;
    const char *message_utf8;
    uint32_t message_length;
    uint32_t reserved0;
} MigoError;

#endif /* MIGO_TYPES_H_ */
