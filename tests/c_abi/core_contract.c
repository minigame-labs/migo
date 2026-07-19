#include <migo/migo.h>

#include <stddef.h>
#include <stdint.h>

#define MIGO_CHECK_PREFIX(TYPE)                                                \
    _Static_assert(offsetof(TYPE, struct_size) == 0, #TYPE " size prefix");   \
    _Static_assert(offsetof(TYPE, abi_version) == 4, #TYPE " ABI prefix")

_Static_assert(MIGO_C_ABI_CANDIDATE == 1, "candidate marker");
/*
 * The macro answers "does a linkable runtime exist for this target", so the
 * assertion checks the rule rather than a constant: desktop Linux ships one,
 * every other target does not. Asserting a fixed 0 here would have to be
 * relaxed the moment any platform gained an implementation, which is exactly
 * when the check is worth having.
 */
#if defined(__linux__) && !defined(__ANDROID__)
_Static_assert(MIGO_C_ABI_HAS_RUNTIME == 1, "desktop Linux ships a runtime");
#else
_Static_assert(MIGO_C_ABI_HAS_RUNTIME == 0, "no runtime outside desktop Linux");
#endif
_Static_assert(MIGO_ABI_VERSION_1 == UINT32_C(1), "ABI version value");
_Static_assert(sizeof(MigoResult) == 4, "fixed-width result");
_Static_assert(MIGO_OK == INT32_C(0), "success value");
_Static_assert(MIGO_ERROR_INVALID_ARGUMENT == -INT32_C(1), "invalid argument value");
_Static_assert(MIGO_ERROR_UNSUPPORTED_ABI == -INT32_C(2), "unsupported ABI value");
_Static_assert(MIGO_ERROR_UNSUPPORTED_PLATFORM == -INT32_C(3),
               "unsupported platform value");
_Static_assert(MIGO_ERROR_UNSUPPORTED_CAPABILITY == -INT32_C(4),
               "unsupported capability value");
_Static_assert(MIGO_ERROR_INVALID_STATE == -INT32_C(5), "invalid state value");
_Static_assert(MIGO_ERROR_WRONG_THREAD == -INT32_C(6), "wrong thread value");
_Static_assert(MIGO_ERROR_STALE_SURFACE == -INT32_C(7), "stale Surface value");
_Static_assert(MIGO_ERROR_CANCELLED == -INT32_C(8), "cancelled value");
_Static_assert(MIGO_ERROR_DISPATCH_REJECTED == -INT32_C(9),
               "dispatch rejection value");
_Static_assert(MIGO_ERROR_OUT_OF_MEMORY == -INT32_C(10), "out-of-memory value");
_Static_assert(MIGO_ERROR_INTERNAL == -INT32_C(11), "internal error value");

_Static_assert(MIGO_ERROR_WOULD_BLOCK == -INT32_C(12), "would-block value");

/* Must match Rust's TouchPoint, which asserts 20 bytes on its side. A mismatch
 * here would corrupt every touch the host sends, silently. */
_Static_assert(sizeof(MigoTouchPoint) == 20, "touch point layout");
_Static_assert(offsetof(MigoTouchPoint, id) == 0, "touch point id offset");
_Static_assert(offsetof(MigoTouchPoint, x) == 4, "touch point x offset");
_Static_assert(offsetof(MigoTouchPoint, y) == 8, "touch point y offset");
_Static_assert(offsetof(MigoTouchPoint, pressure) == 12, "touch point pressure offset");
_Static_assert(offsetof(MigoTouchPoint, flags) == 16, "touch point flags offset");
MIGO_CHECK_PREFIX(MigoTouchEvent);
_Static_assert(MIGO_TOUCH_MAX_POINTS == 10, "inline array capacity");
_Static_assert(MIGO_TOUCH_START == UINT32_C(0), "touch start value");
_Static_assert(MIGO_TOUCH_MOVE == UINT32_C(1), "touch move value");
_Static_assert(MIGO_TOUCH_END == UINT32_C(2), "touch end value");
_Static_assert(MIGO_TOUCH_CANCEL == UINT32_C(3), "touch cancel value");

#ifdef MIGO_ERROR_ALREADY_DETACHED
#error "a consumed attachment cannot safely report already-detached through the old pointer"
#endif

MIGO_CHECK_PREFIX(MigoError);
MIGO_CHECK_PREFIX(MigoEngineConfig);
MIGO_CHECK_PREFIX(MigoSessionConfig);
MIGO_CHECK_PREFIX(MigoPlatformSurfaceDescriptor);
MIGO_CHECK_PREFIX(MigoSurfaceMetrics);
MIGO_CHECK_PREFIX(MigoSurfaceDescriptor);
MIGO_CHECK_PREFIX(MigoHostCallbacks);

_Static_assert(offsetof(MigoSurfaceDescriptor, generation) == 8,
               "generation is naturally aligned");
_Static_assert(offsetof(MigoSurfaceDescriptor, platform_descriptor) == 64,
               "platform payload pointer is append-safe");
_Static_assert(sizeof(MigoSurfaceMetrics) == 48, "metrics v1 layout");

#if UINTPTR_MAX == UINT64_MAX
_Static_assert(sizeof(MigoError) == 32, "LP64 error layout");
_Static_assert(sizeof(MigoSurfaceDescriptor) == 72, "LP64 Surface layout");
_Static_assert(sizeof(MigoHostCallbacks) == 64, "LP64 callback layout");
#elif UINTPTR_MAX == UINT32_MAX
_Static_assert(sizeof(MigoError) == 28, "ILP32 error layout");
_Static_assert(sizeof(MigoSurfaceDescriptor) ==
                   (_Alignof(uint64_t) == 8 ? 72 : 68),
               "ILP32 Surface layout follows the target uint64_t alignment");
_Static_assert(sizeof(MigoHostCallbacks) == 36, "ILP32 callback layout");
#else
#error "unsupported pointer width"
#endif

int migo_core_c_contract(void) {
    MigoEngineConfig engine_config = {0};
    MigoSessionConfig session_config = {0};
    MigoSurfaceDescriptor surface = {0};
    MigoHostCallbacks callbacks = {0};

    engine_config.struct_size = (uint32_t)sizeof(engine_config);
    engine_config.abi_version = MIGO_ABI_VERSION_1;
    session_config.struct_size = (uint32_t)sizeof(session_config);
    session_config.abi_version = MIGO_ABI_VERSION_1;
    surface.struct_size = (uint32_t)sizeof(surface);
    surface.abi_version = MIGO_ABI_VERSION_1;
    callbacks.struct_size = (uint32_t)sizeof(callbacks);
    callbacks.abi_version = MIGO_ABI_VERSION_1;

    MigoResult(MIGO_CALL *attach_fn)(MigoSession *, const MigoSurfaceDescriptor *,
                                     MigoSurfaceAttachment **) =
        &migo_session_attach_surface;
    MigoResult(MIGO_CALL *update_fn)(MigoSurfaceAttachment *,
                                     const MigoSurfaceMetrics *) =
        &migo_surface_update;
    MigoResult(MIGO_CALL *detach_fn)(MigoSurfaceAttachment *) = &migo_surface_detach;
    MigoResult(MIGO_CALL *engine_create_fn)(const MigoEngineConfig *, MigoEngine **) =
        &migo_engine_create;
    MigoResult(MIGO_CALL *engine_destroy_fn)(MigoEngine *) = &migo_engine_destroy;
    MigoResult(MIGO_CALL *session_create_fn)(MigoEngine *, const MigoSessionConfig *,
                                             MigoSession **) = &migo_session_create;
    MigoResult(MIGO_CALL *set_callbacks_fn)(MigoSession *, const MigoHostCallbacks *) =
        &migo_session_set_host_callbacks;
    MigoResult(MIGO_CALL *set_lifecycle_fn)(MigoSession *, MigoLifecycleState) =
        &migo_session_set_lifecycle;
    MigoResult(MIGO_CALL *set_visibility_fn)(MigoSession *, uint8_t) =
        &migo_session_set_visibility;
    MigoResult(MIGO_CALL *set_focus_fn)(MigoSession *, uint8_t) =
        &migo_session_set_focus;
    MigoResult(MIGO_CALL *destroy_fn)(MigoSession *) = &migo_session_destroy;

    return (int)(engine_config.struct_size + session_config.struct_size +
                 surface.struct_size + callbacks.struct_size +
                 (attach_fn != NULL) + (update_fn != NULL) + (detach_fn != NULL) +
                 (engine_create_fn != NULL) + (engine_destroy_fn != NULL) +
                 (session_create_fn != NULL) + (set_callbacks_fn != NULL) +
                 (set_lifecycle_fn != NULL) + (set_visibility_fn != NULL) +
                 (set_focus_fn != NULL) + (destroy_fn != NULL));
}
