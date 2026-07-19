/*
 * Minimal third-party host: opens an X11 window and runs a Migo game through
 * the public C ABI only.
 *
 * This example is the ABI's acceptance test. It includes nothing but the public
 * migo headers, so if it ever needs a private engine detail to work, the ABI is
 * incomplete — which is the feedback we want before v1 is frozen.
 *
 * Build + run: scripts/dev-run-c-host.sh
 */

/* nanosleep is POSIX, not ISO C11, so ask for it explicitly. */
#define _POSIX_C_SOURCE 199309L

#include <migo/migo.h>
#include <migo/platform/x11.h>

#include <X11/Xlib.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#define WINDOW_WIDTH 720
#define WINDOW_HEIGHT 1280

static void sleep_ms(long ms) {
    struct timespec ts;
    ts.tv_sec = ms / 1000;
    ts.tv_nsec = (ms % 1000) * 1000000L;
    nanosleep(&ts, NULL);
}

/* ---- Host callbacks. The engine never calls these directly: it hands a task
 * to our dispatcher, and we run it where we choose. This example runs it
 * inline, which is legal — "exactly once, inline or later". ---- */
static MigoResult MIGO_CALL dispatch_inline(void *dispatcher_context, MigoTaskFn task,
                                            void *task_context) {
    (void)dispatcher_context;
    task(task_context);
    return MIGO_OK;
}

static void MIGO_CALL on_ready(void *user_data, MigoSession *session) {
    (void)user_data;
    (void)session;
    printf("[c-host] callback: content is ready\n");
    fflush(stdout);
}

static void MIGO_CALL on_error(void *user_data, MigoSession *session,
                               const MigoError *error) {
    (void)user_data;
    (void)session;
    fprintf(stderr, "[c-host] callback: engine error %d: %.*s\n", (int)error->code,
            (int)error->message_length, error->message_utf8);
    fflush(stderr);
}

static void MIGO_CALL on_exit_requested(void *user_data, MigoSession *session) {
    (void)user_data;
    (void)session;
    printf("[c-host] callback: content asked to exit\n");
    fflush(stdout);
}

static int fail(const char *what, MigoResult result) {
    fprintf(stderr, "[c-host] %s failed: %d\n", what, (int)result);
    return 1;
}

/*
 * X11 reports physical pixels; the ABI takes CSS pixels. This one constant is
 * used both for the attach descriptor and for converting input, so the two
 * cannot drift -- sending physical pixels is what makes a game render correctly
 * and respond in the wrong place.
 */
static const float SCALE_FACTOR = 1.0f;

/* Maps one X11 pointer position onto a single-point touch event (id 0), which
 * is what wx content listens for; there is no separate mouse event model. */
static void send_touch(MigoSession *session, MigoTouchType type, int x, int y,
                       int64_t timestamp_ms) {
    int removed = (type == MIGO_TOUCH_END || type == MIGO_TOUCH_CANCEL);

    MigoTouchPoint point;
    memset(&point, 0, sizeof point);
    point.id = 0;
    point.x = (float)x / SCALE_FACTOR;
    point.y = (float)y / SCALE_FACTOR;
    point.pressure = removed ? 0.0f : 1.0f;
    point.flags = MIGO_TOUCH_FLAG_CHANGED;
    if (removed) point.flags |= MIGO_TOUCH_FLAG_REMOVED;

    MigoTouchEvent event;
    memset(&event, 0, sizeof event);
    event.struct_size = (uint32_t)sizeof event;
    event.abi_version = MIGO_ABI_VERSION_CURRENT;
    event.type = type;
    event.point_count = 1;
    event.timestamp_ms = timestamp_ms;
    event.points = &point;

    MigoResult result = migo_session_send_touch(session, &event);
    if (result != MIGO_OK) {
        fprintf(stderr, "[c-host] touch not delivered: %d\n", (int)result);
    }
}

int main(int argc, char **argv) {
    const char *files_dir = (argc > 1) ? argv[1] : "/tmp/migo-c-host/files";
    const char *content_id = (argc > 2) ? argv[2] : "c-host-demo";
    const int seconds = (argc > 3) ? atoi(argv[3]) : 10;

    /* ---- Host-owned window. Migo never creates one. ---- */
    if (!XInitThreads()) {
        fprintf(stderr, "[c-host] XInitThreads failed\n");
        return 1;
    }
    Display *display = XOpenDisplay(NULL);
    if (!display) {
        fprintf(stderr, "[c-host] XOpenDisplay failed (DISPLAY=%s)\n",
                getenv("DISPLAY") ? getenv("DISPLAY") : "(unset)");
        return 1;
    }
    int screen = DefaultScreen(display);
    Window window = XCreateSimpleWindow(display, RootWindow(display, screen), 0, 0,
                                        WINDOW_WIDTH, WINDOW_HEIGHT, 0,
                                        BlackPixel(display, screen),
                                        BlackPixel(display, screen));
    XStoreName(display, window, "migo-c-host");
    XSelectInput(display, window,
                 StructureNotifyMask | ButtonPressMask | ButtonReleaseMask |
                     PointerMotionMask | LeaveWindowMask);
    Atom wm_delete = XInternAtom(display, "WM_DELETE_WINDOW", False);
    XSetWMProtocols(display, window, &wm_delete, 1);
    XMapWindow(display, window);
    XFlush(display);

    /* ---- Engine: the host names every storage root. ---- */
    char cache_dir[512];
    char code_cache_dir[512];
    snprintf(cache_dir, sizeof(cache_dir), "%s/../cache", files_dir);
    snprintf(code_cache_dir, sizeof(code_cache_dir), "%s/../code-cache", files_dir);

    MigoEngineConfig engine_config;
    memset(&engine_config, 0, sizeof(engine_config));
    engine_config.struct_size = (uint32_t)sizeof(engine_config);
    engine_config.abi_version = MIGO_ABI_VERSION_CURRENT;
    /* Development example: the bundled game carries no signing receipt. */
    engine_config.flags = MIGO_ENGINE_FLAG_ALLOW_UNSIGNED_CONTENT;
    engine_config.files_dir_utf8 = files_dir;
    engine_config.cache_dir_utf8 = cache_dir;
    engine_config.code_cache_dir_utf8 = code_cache_dir;

    MigoEngine *engine = NULL;
    MigoResult result = migo_engine_create(&engine_config, &engine);
    if (result != MIGO_OK) return fail("migo_engine_create", result);

    MigoSessionConfig session_config;
    memset(&session_config, 0, sizeof(session_config));
    session_config.struct_size = (uint32_t)sizeof(session_config);
    session_config.abi_version = MIGO_ABI_VERSION_CURRENT;
    session_config.flags = MIGO_SESSION_FLAG_NONE;

    MigoSession *session = NULL;
    result = migo_session_create(engine, &session_config, &session);
    if (result != MIGO_OK) return fail("migo_session_create", result);

    /* ---- Callbacks install once, before the first attach. ---- */
    MigoHostCallbacks host_callbacks;
    memset(&host_callbacks, 0, sizeof(host_callbacks));
    host_callbacks.struct_size = (uint32_t)sizeof(host_callbacks);
    host_callbacks.abi_version = MIGO_ABI_VERSION_CURRENT;
    host_callbacks.dispatch = dispatch_inline;
    host_callbacks.on_ready = on_ready;
    host_callbacks.on_error = on_error;
    host_callbacks.on_exit_requested = on_exit_requested;

    result = migo_session_set_host_callbacks(session, &host_callbacks);
    if (result != MIGO_OK) return fail("migo_session_set_host_callbacks", result);

    /* ---- Hand the window over as a strongly typed platform descriptor. ---- */
    MigoX11WindowDescriptor x11;
    memset(&x11, 0, sizeof(x11));
    x11.struct_size = (uint32_t)sizeof(x11);
    x11.abi_version = MIGO_ABI_VERSION_CURRENT;
    x11.platform_kind = MIGO_PLATFORM_X11_WINDOW;
    x11.flags = MIGO_PLATFORM_DESCRIPTOR_FLAG_NONE;
    x11.display = display;
    x11.window = (uintptr_t)window;
    x11.screen = screen;

    MigoSurfaceDescriptor surface;
    memset(&surface, 0, sizeof(surface));
    surface.struct_size = (uint32_t)sizeof(surface);
    surface.abi_version = MIGO_ABI_VERSION_CURRENT;
    surface.generation = 1;
    surface.platform_kind = MIGO_PLATFORM_X11_WINDOW;
    surface.flags = MIGO_SURFACE_DESCRIPTOR_FLAG_NONE;
    surface.width_pixels = WINDOW_WIDTH;
    surface.height_pixels = WINDOW_HEIGHT;
    surface.scale_factor = SCALE_FACTOR;
    surface.color_space = MIGO_COLOR_SPACE_SRGB;
    surface.alpha_mode = MIGO_ALPHA_MODE_OPAQUE;
    surface.preferred_presentation_mode = MIGO_PRESENTATION_MODE_DEFAULT;
    surface.capability_flags = MIGO_SURFACE_CAPABILITY_NONE;
    surface.platform_descriptor_size = (uint32_t)sizeof(x11);
    surface.platform_descriptor = &x11;

    MigoSurfaceAttachment *attachment = NULL;
    result = migo_session_attach_surface(session, &surface, &attachment);
    if (result != MIGO_OK) return fail("migo_session_attach_surface", result);

    MigoContentDescriptor content;
    memset(&content, 0, sizeof(content));
    content.struct_size = (uint32_t)sizeof(content);
    content.abi_version = MIGO_ABI_VERSION_CURRENT;
    content.flags = MIGO_CONTENT_FLAG_NONE;
    content.content_id_utf8 = content_id;
    content.entry_utf8 = "game.js";

    result = migo_session_load_content(session, &content);
    if (result != MIGO_OK) return fail("migo_session_load_content", result);

    printf("[c-host] running '%s' for %ds in window 0x%lx\n", content_id, seconds,
           (unsigned long)window);
    fflush(stdout);

    /* ---- The host owns the event loop; Migo renders on its own thread. ---- */
    int pressed = 0;
    for (int elapsed = 0; elapsed < seconds * 1000; elapsed += 16) {
        while (XPending(display) > 0) {
            XEvent event;
            XNextEvent(display, &event);
            if (event.type == ClientMessage &&
                (Atom)event.xclient.data.l[0] == wm_delete) {
                printf("[c-host] window close requested\n");
                elapsed = seconds * 1000;
                break;
            }
            switch (event.type) {
            case ButtonPress:
                if (event.xbutton.button == Button1) {
                    pressed = 1;
                    send_touch(session, MIGO_TOUCH_START, event.xbutton.x,
                               event.xbutton.y, (int64_t)event.xbutton.time);
                }
                break;
            case MotionNotify:
                /* Only while a button is down: wx content has no hover concept,
                 * so free motion would be a stream of events no game reads. */
                if (pressed) {
                    send_touch(session, MIGO_TOUCH_MOVE, event.xmotion.x,
                               event.xmotion.y, (int64_t)event.xmotion.time);
                }
                break;
            case ButtonRelease:
                if (event.xbutton.button == Button1 && pressed) {
                    pressed = 0;
                    send_touch(session, MIGO_TOUCH_END, event.xbutton.x,
                               event.xbutton.y, (int64_t)event.xbutton.time);
                }
                break;
            case LeaveNotify:
                /* The pointer left with a button still down, so the release will
                 * never arrive: cancel rather than strand the touch. */
                if (pressed) {
                    pressed = 0;
                    send_touch(session, MIGO_TOUCH_CANCEL, event.xcrossing.x,
                               event.xcrossing.y, (int64_t)event.xcrossing.time);
                }
                break;
            default:
                break;
            }
        }
        sleep_ms(16);
    }

    /* ---- Teardown in the order the headers require. ---- */
    result = migo_surface_detach(attachment);
    if (result != MIGO_OK) return fail("migo_surface_detach", result);
    result = migo_session_destroy(session);
    if (result != MIGO_OK) return fail("migo_session_destroy", result);
    result = migo_engine_destroy(engine);
    if (result != MIGO_OK) return fail("migo_engine_destroy", result);

    XDestroyWindow(display, window);
    XCloseDisplay(display);
    printf("[c-host] done\n");
    return 0;
}
