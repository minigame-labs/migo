#include <migo/migo.h>

#include <cstddef>
#include <cstdint>
#include <type_traits>

#define MIGO_CHECK_CXX_RECORD(TYPE)                                            \
    static_assert(std::is_standard_layout<TYPE>::value, #TYPE " standard layout"); \
    static_assert(std::is_trivially_copyable<TYPE>::value,                    \
                  #TYPE " trivially copyable");                              \
    static_assert(offsetof(TYPE, struct_size) == 0, #TYPE " size prefix");   \
    static_assert(offsetof(TYPE, abi_version) == 4, #TYPE " ABI prefix")

static_assert(MIGO_C_ABI_CANDIDATE == 1, "candidate marker");
/*
 * The macro answers "does a linkable runtime exist for this target", so the
 * assertion checks the rule rather than a constant: desktop Linux ships one,
 * every other target does not. Asserting a fixed 0 here would have to be
 * relaxed the moment any platform gained an implementation, which is exactly
 * when the check is worth having.
 */
#if defined(__linux__) && !defined(__ANDROID__)
static_assert(MIGO_C_ABI_HAS_RUNTIME == 1, "desktop Linux ships a runtime");
#else
static_assert(MIGO_C_ABI_HAS_RUNTIME == 0, "no runtime outside desktop Linux");
#endif
static_assert(sizeof(MigoResult) == 4, "fixed-width result");

static_assert(MIGO_ERROR_WOULD_BLOCK == -INT32_C(12), "would-block value");

/* Must match Rust's TouchPoint, which asserts 20 bytes on its side. A mismatch
 * here would corrupt every touch the host sends, silently. */
static_assert(sizeof(MigoTouchPoint) == 20, "touch point layout");
static_assert(offsetof(MigoTouchPoint, id) == 0, "touch point id offset");
static_assert(offsetof(MigoTouchPoint, x) == 4, "touch point x offset");
static_assert(offsetof(MigoTouchPoint, y) == 8, "touch point y offset");
static_assert(offsetof(MigoTouchPoint, pressure) == 12, "touch point pressure offset");
static_assert(offsetof(MigoTouchPoint, flags) == 16, "touch point flags offset");
MIGO_CHECK_CXX_RECORD(MigoTouchEvent);
static_assert(MIGO_TOUCH_MAX_POINTS == 10, "inline array capacity");
static_assert(MIGO_TOUCH_START == UINT32_C(0), "touch start value");
static_assert(MIGO_TOUCH_MOVE == UINT32_C(1), "touch move value");
static_assert(MIGO_TOUCH_END == UINT32_C(2), "touch end value");
static_assert(MIGO_TOUCH_CANCEL == UINT32_C(3), "touch cancel value");

MIGO_CHECK_CXX_RECORD(MigoError);
MIGO_CHECK_CXX_RECORD(MigoEngineConfig);
MIGO_CHECK_CXX_RECORD(MigoSessionConfig);
MIGO_CHECK_CXX_RECORD(MigoPlatformSurfaceDescriptor);
MIGO_CHECK_CXX_RECORD(MigoSurfaceMetrics);
MIGO_CHECK_CXX_RECORD(MigoSurfaceDescriptor);
MIGO_CHECK_CXX_RECORD(MigoHostCallbacks);

using AttachFn = MigoResult(MIGO_CALL *)(MigoSession *,
                                         const MigoSurfaceDescriptor *,
                                         MigoSurfaceAttachment **);
using UpdateFn = MigoResult(MIGO_CALL *)(MigoSurfaceAttachment *,
                                         const MigoSurfaceMetrics *);
using BeginDetachFn = MigoResult(MIGO_CALL *)(MigoSurfaceAttachment *,
                                              MigoSurfaceRelease **);
using ReleaseQueryFn = MigoResult(MIGO_CALL *)(const MigoSurfaceRelease *,
                                               MigoSurfaceReleaseStatus *);
using ReleaseDestroyFn = MigoResult(MIGO_CALL *)(MigoSurfaceRelease *);
using EngineCreateFn = MigoResult(MIGO_CALL *)(const MigoEngineConfig *,
                                               MigoEngine **);
using EngineDestroyFn = MigoResult(MIGO_CALL *)(MigoEngine *);
using SessionCreateFn = MigoResult(MIGO_CALL *)(MigoEngine *,
                                                const MigoSessionConfig *,
                                                MigoSession **);
using SetCallbacksFn = MigoResult(MIGO_CALL *)(MigoSession *,
                                               const MigoHostCallbacks *);
using SetLifecycleFn = MigoResult(MIGO_CALL *)(MigoSession *, MigoLifecycleState);
using SetVisibilityFn = MigoResult(MIGO_CALL *)(MigoSession *, std::uint8_t);
using SetFocusFn = MigoResult(MIGO_CALL *)(MigoSession *, std::uint8_t);
using SessionDestroyFn = MigoResult(MIGO_CALL *)(MigoSession *);

static_assert(std::is_same<decltype(&migo_session_attach_surface), AttachFn>::value,
              "attach declaration");
static_assert(std::is_same<decltype(&migo_surface_update), UpdateFn>::value,
              "update declaration");
static_assert(std::is_same<decltype(&migo_surface_begin_detach), BeginDetachFn>::value,
              "begin detach declaration");
static_assert(std::is_same<decltype(&migo_surface_release_query), ReleaseQueryFn>::value,
              "release query declaration");
static_assert(std::is_same<decltype(&migo_surface_release_destroy), ReleaseDestroyFn>::value,
              "release destroy declaration");
static_assert(std::is_same<decltype(&migo_engine_create), EngineCreateFn>::value,
              "engine create declaration");
static_assert(std::is_same<decltype(&migo_engine_destroy), EngineDestroyFn>::value,
              "engine destroy declaration");
static_assert(std::is_same<decltype(&migo_session_create), SessionCreateFn>::value,
              "session create declaration");
static_assert(
    std::is_same<decltype(&migo_session_set_host_callbacks), SetCallbacksFn>::value,
    "callback declaration");
static_assert(
    std::is_same<decltype(&migo_session_set_lifecycle), SetLifecycleFn>::value,
    "lifecycle declaration");
static_assert(
    std::is_same<decltype(&migo_session_set_visibility), SetVisibilityFn>::value,
    "visibility declaration");
static_assert(std::is_same<decltype(&migo_session_set_focus), SetFocusFn>::value,
              "focus declaration");
static_assert(std::is_same<decltype(&migo_session_destroy), SessionDestroyFn>::value,
              "session destroy declaration");

int migo_core_cpp_contract() {
    MigoEngineConfig engine_config{};
    MigoSessionConfig session_config{};
    MigoSurfaceDescriptor surface{};
    MigoHostCallbacks callbacks{};

    engine_config.struct_size = static_cast<std::uint32_t>(sizeof(engine_config));
    engine_config.abi_version = MIGO_ABI_VERSION_1;
    session_config.struct_size = static_cast<std::uint32_t>(sizeof(session_config));
    session_config.abi_version = MIGO_ABI_VERSION_1;
    surface.struct_size = static_cast<std::uint32_t>(sizeof(surface));
    surface.abi_version = MIGO_ABI_VERSION_1;
    callbacks.struct_size = static_cast<std::uint32_t>(sizeof(callbacks));
    callbacks.abi_version = MIGO_ABI_VERSION_1;

    return static_cast<int>(engine_config.struct_size + session_config.struct_size +
                            surface.struct_size + callbacks.struct_size);
}
