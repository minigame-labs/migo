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

static int fail(const char *what, MigoResult result) {
    fprintf(stderr, "[c-host] %s failed: %d\n", what, (int)result);
    return 1;
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
    XSelectInput(display, window, StructureNotifyMask);
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
    engine_config.flags = MIGO_ENGINE_FLAG_NONE;
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
    surface.scale_factor = 1.0f;
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
