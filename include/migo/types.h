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
 * CMake integration. Android embeds through the Java/JNI SDK and exports no
 * migo_* symbols, so the answer there is still no -- a host must not compile
 * against these declarations expecting to link.
 *
 * This says a runtime exists, not that the ABI is frozen; MIGO_C_ABI_CANDIDATE
 * remains 1 until the README's blockers are closed.
 */
#if defined(__linux__) && !defined(__ANDROID__)
#define MIGO_C_ABI_HAS_RUNTIME 1
#else
#define MIGO_C_ABI_HAS_RUNTIME 0
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
