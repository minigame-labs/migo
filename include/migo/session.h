#ifndef MIGO_SESSION_H_
#define MIGO_SESSION_H_

#include <migo/surface.h>

typedef uint64_t MigoEngineFlags;
#define MIGO_ENGINE_FLAG_NONE UINT64_C(0)
/*
 * Run content that carries no signing receipt. Development and testing only:
 * production hosts leave this clear so unsigned content is refused. It is an
 * explicit opt-in rather than a default because silently accepting unsigned
 * content is exactly the failure a signing check exists to prevent.
 */
#define MIGO_ENGINE_FLAG_ALLOW_UNSIGNED_CONTENT (UINT64_C(1) << 0)

typedef uint64_t MigoSessionFlags;
#define MIGO_SESSION_FLAG_NONE UINT64_C(0)

/*
 * Storage roots belong to the host: Migo reads and writes only underneath the
 * directories given here and never invents a location of its own. All three are
 * NUL-terminated UTF-8, borrowed for the duration of migo_engine_create, and
 * copied before it returns. Directories are created if missing.
 */
typedef struct MigoEngineConfig {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoEngineFlags flags;
    uint32_t reserved0;
    const char *files_dir_utf8;
    const char *cache_dir_utf8;
    const char *code_cache_dir_utf8;
} MigoEngineConfig;

typedef struct MigoSessionConfig {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoSessionFlags flags;
} MigoSessionConfig;

typedef uint32_t MigoContentFlags;
#define MIGO_CONTENT_FLAG_NONE UINT32_C(0)

/*
 * Names content the host has already installed under the engine's files
 * directory, in the same layout the platform SDKs install into
 * (<files_dir>/migo/games/<content_id>/code/<entry>). Both strings are
 * NUL-terminated UTF-8, borrowed for the duration of the call.
 *
 * Loading from an arbitrary host-chosen directory is deliberately not in v1:
 * it would make the engine's content resolution part of the frozen ABI. A
 * host-supplied bundle root is the natural v1.1 extension, added as a new
 * flagged field rather than by changing this struct's meaning.
 */
typedef struct MigoContentDescriptor {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoContentFlags flags;
    uint32_t reserved0;
    const char *content_id_utf8;
    const char *entry_utf8;
} MigoContentDescriptor;

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
/*
 * The engine asks the host to schedule exactly one frame.
 *
 * A host that installs this owns frame pacing: arm one callback with whatever
 * its platform provides -- AChoreographer on Android, the compositor's frame
 * callback on Wayland -- and call migo_session_notify_vsync when it fires. One
 * request means one frame; the engine asks again when it wants another.
 *
 * A host that leaves this NULL is saying it does not drive frames, and the
 * engine paces itself. That is the right answer for a host with no display
 * synchronisation to offer, and the wrong one where the platform has a vsync
 * signal, because self-pacing cannot align to the display.
 *
 * Delivered through the host's dispatcher like every other callback, so it
 * never runs on an engine thread.
 */
typedef void(MIGO_CALL *MigoOnRequestFrameFn)(void *user_data,
                                              MigoSession *session);
typedef void(MIGO_CALL *MigoOnSurfaceLostFn)(void *user_data,
                                             MigoSession *session,
                                             uint64_t generation,
                                             MigoSurfaceLossReason reason);
/*
 * Optional wakeup after the named retired Surface reaches RELEASED. The
 * callback is an edge only: rejection, cancellation, or delayed dispatch does
 * not change release state. Hosts must use migo_surface_release_query as the
 * authoritative level check before destroying the native resource.
 */
typedef void(MIGO_CALL *MigoOnSurfaceReleasedFn)(void *user_data,
                                                 MigoSession *session,
                                                 uint64_t generation);

typedef uint32_t MigoKeyboardFlags;
#define MIGO_KEYBOARD_FLAG_NONE UINT32_C(0)
/* The field accepts more than one line. */
#define MIGO_KEYBOARD_FLAG_MULTIPLE (UINT32_C(1) << 0)
/* Keep the keyboard up after confirm instead of dismissing it. */
#define MIGO_KEYBOARD_FLAG_CONFIRM_HOLD (UINT32_C(1) << 1)

typedef uint32_t MigoKeyboardConfirmType;
#define MIGO_KEYBOARD_CONFIRM_DONE UINT32_C(0)
#define MIGO_KEYBOARD_CONFIRM_NEXT UINT32_C(1)
#define MIGO_KEYBOARD_CONFIRM_SEARCH UINT32_C(2)
#define MIGO_KEYBOARD_CONFIRM_GO UINT32_C(3)
#define MIGO_KEYBOARD_CONFIRM_SEND UINT32_C(4)

typedef uint32_t MigoKeyboardType;
#define MIGO_KEYBOARD_TYPE_TEXT UINT32_C(0)
#define MIGO_KEYBOARD_TYPE_NUMBER UINT32_C(1)

/*
 * What content asked for when it opened the keyboard.
 *
 * The struct and default_value_utf8 are borrowed for the duration of the
 * callback only; a host that needs them afterwards copies them.
 * default_value_utf8 is length-delimited and need not be NUL-terminated.
 */
typedef struct MigoKeyboardShowOptions {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoKeyboardFlags flags;
    uint32_t max_length;
    MigoKeyboardConfirmType confirm_type;
    MigoKeyboardType keyboard_type;
    const char *default_value_utf8;
    uint32_t default_value_length;
    uint32_t reserved0;
} MigoKeyboardShowOptions;

MIGO_STATIC_ASSERT(offsetof(MigoKeyboardShowOptions, struct_size) == 0,
                   "every versioned struct must begin with struct_size");
#if MIGO_LP64
MIGO_STATIC_ASSERT(sizeof(MigoKeyboardShowOptions) == 40,
                   "MigoKeyboardShowOptions LP64 size changed");
MIGO_STATIC_ASSERT(offsetof(MigoKeyboardShowOptions, default_value_utf8) == 24,
                   "MigoKeyboardShowOptions.default_value_utf8 moved");
#endif

/*
 * The soft keyboard is a capability the host supplies, not one Migo has.
 *
 * Install all three or none: a host that can show a keyboard but not hide it
 * strands it on screen with no event that corrects the state, so installing a
 * subset returns MIGO_ERROR_INVALID_ARGUMENT. A host that installs none simply
 * has no keyboard capability, and content's wx.showKeyboard reports failure.
 *
 * value_utf8 in the update callback is the whole current text -- content
 * correcting the value -- length-delimited and borrowed for the call.
 */
typedef void(MIGO_CALL *MigoOnShowKeyboardFn)(void *user_data, MigoSession *session,
                                              const MigoKeyboardShowOptions *options);
typedef void(MIGO_CALL *MigoOnHideKeyboardFn)(void *user_data, MigoSession *session);
typedef void(MIGO_CALL *MigoOnUpdateKeyboardFn)(void *user_data, MigoSession *session,
                                                const char *value_utf8, uint32_t value_length);

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
    /* Appended: a smaller struct_size from an older host simply omits it, and
     * that host is then engine-paced, which is the pre-existing behaviour. */
    MigoOnRequestFrameFn on_request_frame;
    /* Appended: the soft keyboard, all three or none. A host that installs none
     * has no keyboard capability, which is the pre-existing behaviour. */
    MigoOnShowKeyboardFn on_show_keyboard;
    MigoOnHideKeyboardFn on_hide_keyboard;
    MigoOnUpdateKeyboardFn on_update_keyboard;
    /* Appended: optional event-loop wakeup for asynchronous Surface release.
     * Older hosts continue to poll the release observer. */
    MigoOnSurfaceReleasedFn on_surface_released;
} MigoHostCallbacks;

MIGO_STATIC_ASSERT(offsetof(MigoHostCallbacks, struct_size) == 0,
                   "every versioned struct must begin with struct_size");
#if MIGO_LP64
MIGO_STATIC_ASSERT(sizeof(MigoHostCallbacks) == 104, "MigoHostCallbacks LP64 size changed");
MIGO_STATIC_ASSERT(offsetof(MigoHostCallbacks, dispatch) == 24, "MigoHostCallbacks.dispatch moved");
MIGO_STATIC_ASSERT(offsetof(MigoHostCallbacks, on_request_frame) == 64,
                   "MigoHostCallbacks.on_request_frame moved");
MIGO_STATIC_ASSERT(offsetof(MigoHostCallbacks, on_update_keyboard) == 88,
                   "MigoHostCallbacks.on_update_keyboard moved");
MIGO_STATIC_ASSERT(offsetof(MigoHostCallbacks, on_surface_released) == 96,
                   "MigoHostCallbacks.on_surface_released moved");
#endif

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

/*
 * Starts evaluating the named content. Argument and state problems are reported
 * synchronously; failures raised while the content runs arrive through
 * on_error, because by then the caller's stack is long gone. A Session loads
 * content once: a second call returns MIGO_ERROR_INVALID_STATE.
 */
MIGO_API MigoResult MIGO_CALL migo_session_load_content(
    MigoSession *session,
    const MigoContentDescriptor *content);

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
 * Report that a frame boundary arrived, in response to on_request_frame.
 *
 * frame_time_nanos is the platform's frame timestamp -- AChoreographer's
 * callback argument on Android. Calling this without having been asked is
 * harmless but pointless: the engine renders at most one frame per request.
 *
 * Returns MIGO_ERROR_INVALID_STATE when no surface is attached.
 */
MIGO_API MigoResult MIGO_CALL
migo_session_notify_vsync(MigoSession *session, int64_t frame_time_nanos);

/*
 * Queued callbacks are canceled before return. Reentrant destruction from the
 * current callback invalidates the session immediately and lets that stack
 * unwind without starting another user callback. Destruction refuses with
 * MIGO_ERROR_INVALID_STATE while a Surface transition is active, an attachment
 * is still live, or any retired Surface is still PENDING; ownership remains
 * with the caller and the Session can be retried after release. MIGO_OK consumes
 * and releases the Session handle; its pointer is invalid afterward.
 */
MIGO_API MigoResult MIGO_CALL migo_session_destroy(MigoSession *session);

MIGO_END_DECLS

#endif /* MIGO_SESSION_H_ */
