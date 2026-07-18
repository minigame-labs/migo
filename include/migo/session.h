#ifndef MIGO_SESSION_H_
#define MIGO_SESSION_H_

#include <migo/surface.h>

typedef uint64_t MigoEngineFlags;
#define MIGO_ENGINE_FLAG_NONE UINT64_C(0)

typedef uint64_t MigoSessionFlags;
#define MIGO_SESSION_FLAG_NONE UINT64_C(0)

typedef struct MigoEngineConfig {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoEngineFlags flags;
} MigoEngineConfig;

typedef struct MigoSessionConfig {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoSessionFlags flags;
} MigoSessionConfig;

typedef uint32_t MigoLifecycleState;
#define MIGO_LIFECYCLE_CREATED UINT32_C(0)
#define MIGO_LIFECYCLE_RUNNING UINT32_C(1)
#define MIGO_LIFECYCLE_PAUSED UINT32_C(2)

MIGO_BEGIN_DECLS

typedef void(MIGO_CALL *MigoTaskFn)(void *task_context);

/*
 * MIGO_OK transfers the task to the dispatcher, which must invoke it exactly
 * once (inline or later). Any error leaves task ownership with Migo.
 */
typedef MigoResult(MIGO_CALL *MigoDispatchFn)(void *dispatcher_context,
                                              MigoTaskFn task,
                                              void *task_context);

typedef void(MIGO_CALL *MigoOnReadyFn)(void *user_data,
                                       MigoSession *session);
typedef void(MIGO_CALL *MigoOnErrorFn)(void *user_data,
                                       MigoSession *session,
                                       const MigoError *error);
typedef void(MIGO_CALL *MigoOnExitRequestedFn)(void *user_data,
                                               MigoSession *session);
typedef void(MIGO_CALL *MigoOnSurfaceLostFn)(void *user_data,
                                             MigoSession *session,
                                             uint64_t generation,
                                             MigoSurfaceLossReason reason);

/*
 * The implementation copies known fields covered by struct_size. A non-null
 * callback requires a non-null dispatcher. User callbacks run without Migo
 * engine/session/attachment locks and may re-enter detach or destroy. Callback
 * configuration can be installed successfully only once per Session and only
 * before its first Surface attach or RUNNING transition; later attempts return
 * MIGO_ERROR_INVALID_STATE. This prevents queued tasks from observing replaced
 * function pointers or user_data.
 */
typedef struct MigoHostCallbacks {
    uint32_t struct_size;
    uint32_t abi_version;
    void *user_data;
    void *dispatcher_data;
    MigoDispatchFn dispatch;
    MigoOnReadyFn on_ready;
    MigoOnErrorFn on_error;
    MigoOnExitRequestedFn on_exit_requested;
    MigoOnSurfaceLostFn on_surface_lost;
} MigoHostCallbacks;

MIGO_API MigoResult MIGO_CALL
migo_engine_create(const MigoEngineConfig *config, MigoEngine **out_engine);

/*
 * All child sessions must be destroyed before engine destruction. MIGO_OK
 * consumes and releases the Engine handle; the pointer is invalid afterward.
 */
MIGO_API MigoResult MIGO_CALL migo_engine_destroy(MigoEngine *engine);

MIGO_API MigoResult MIGO_CALL migo_session_create(
    MigoEngine *engine,
    const MigoSessionConfig *config,
    MigoSession **out_session);

MIGO_API MigoResult MIGO_CALL migo_session_set_host_callbacks(
    MigoSession *session,
    const MigoHostCallbacks *callbacks);

MIGO_API MigoResult MIGO_CALL migo_session_set_lifecycle(
    MigoSession *session,
    MigoLifecycleState state);

MIGO_API MigoResult MIGO_CALL
migo_session_set_visibility(MigoSession *session, uint8_t visible);

MIGO_API MigoResult MIGO_CALL
migo_session_set_focus(MigoSession *session, uint8_t focused);

/*
 * Queued callbacks are canceled before return. Reentrant destruction from the
 * current callback invalidates the session immediately and lets that stack
 * unwind without starting another user callback. Destruction also consumes
 * every still-live SurfaceAttachment; all caller-held attachment pointers are
 * invalid afterward. MIGO_OK consumes and releases the Session handle; its
 * pointer is invalid afterward.
 */
MIGO_API MigoResult MIGO_CALL migo_session_destroy(MigoSession *session);

MIGO_END_DECLS

#endif /* MIGO_SESSION_H_ */
