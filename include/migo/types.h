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
 * This source-visible ABI is still a design candidate: several freeze blockers
 * listed in README.md are open, notably input contracts, asynchronous request
 * identity, capability queries, and an Android implementation of this same
 * contract. Do not treat it as stable.
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
 * cross-compiles it, and examples/c-host/android is a NativeActivity with no
 * Java of its own that links it and drives surface attach, lifecycle, touch,
 * the soft keyboard, physical keys, IME composition and gamepads on device.
 * The separate libmigo.so that the Java/JNI SDK ships still exports no migo_*
 * symbols -- that is a different artifact, and this macro is about whether a
 * linkable runtime exists, not about which artifact carries it.
 *
 * What Android does NOT have yet is packaging: no pkg-config, no CMake package,
 * no versioned shared object. A host links the static library from the source
 * tree. That gap is tracked in README.md, not here.
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
 * so testing __linux__ alone silently answers for three different ABIs at once,
 * and would claim a runtime on OpenHarmony, where none is built.
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

#if MIGO_PLATFORM_IS_LINUX_GNU || MIGO_PLATFORM_IS_ANDROID
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

/* LP64 is the only shape that ships today (linux-x86_64, aarch64-linux-android).
 * Sizes that contain a pointer are asserted only there; ILP32 needs its own
 * numbers, and asserting invented ones would be worse than asserting none. */
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
