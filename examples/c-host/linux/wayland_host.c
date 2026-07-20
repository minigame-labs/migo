/*
 * The same third-party host, on Wayland.
 *
 * Kept separate from main.c rather than folded in behind #ifdefs: these files
 * are documentation as much as they are tests, and a host author reading one
 * should not have to skip past the other window system.
 *
 * What Wayland changes, and what it does not: Migo still owns nothing. The host
 * connects to the compositor, creates the wl_surface, gives it a role through
 * xdg-shell, and runs the dispatch loop. Migo receives two opaque pointers and
 * hands them to EGL. That is the same contract the X11 host has -- only the
 * handles differ.
 *
 * The one Wayland-specific obligation is the initial commit: a wl_surface has
 * no size until the compositor configures it, and a surface must be committed
 * without a buffer once before that configure arrives. Attaching before the
 * configure gives EGL a surface the compositor has not agreed to show.
 */

#define _POSIX_C_SOURCE 199309L

#include <migo/migo.h>
#include <migo/platform/wayland.h>

#include <wayland-client.h>
#include "xdg-shell-client-protocol.h"

#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <poll.h>
#include <time.h>

#define WINDOW_WIDTH 720
#define WINDOW_HEIGHT 1280

struct wl_host {
    struct wl_display *display;
    struct wl_compositor *compositor;
    struct xdg_wm_base *wm_base;
    struct wl_surface *surface;
    struct xdg_surface *xdg_surface;
    struct xdg_toplevel *toplevel;
    int configured;
};

/* ---- Registry: bind only what a window needs. ---- */
static void registry_global(void *data, struct wl_registry *registry, uint32_t name,
                            const char *interface, uint32_t version) {
    struct wl_host *h = data;
    (void)version;
    if (strcmp(interface, wl_compositor_interface.name) == 0) {
        h->compositor = wl_registry_bind(registry, name, &wl_compositor_interface, 4);
    } else if (strcmp(interface, xdg_wm_base_interface.name) == 0) {
        h->wm_base = wl_registry_bind(registry, name, &xdg_wm_base_interface, 1);
    }
}

static void registry_global_remove(void *data, struct wl_registry *registry, uint32_t name) {
    (void)data;
    (void)registry;
    (void)name;
}

static const struct wl_registry_listener registry_listener = {registry_global,
                                                              registry_global_remove};

/* The compositor pings to check we are alive; not answering gets the window
 * killed as unresponsive, which looks exactly like a renderer hang. */
static void wm_base_ping(void *data, struct xdg_wm_base *wm_base, uint32_t serial) {
    (void)data;
    xdg_wm_base_pong(wm_base, serial);
}

static const struct xdg_wm_base_listener wm_base_listener = {wm_base_ping};

static void xdg_surface_configure(void *data, struct xdg_surface *xdg_surface, uint32_t serial) {
    struct wl_host *h = data;
    xdg_surface_ack_configure(xdg_surface, serial);
    h->configured = 1;
}

static const struct xdg_surface_listener xdg_surface_listener = {xdg_surface_configure};

static void toplevel_configure(void *data, struct xdg_toplevel *toplevel, int32_t width,
                               int32_t height, struct wl_array *states) {
    (void)data;
    (void)toplevel;
    (void)width;
    (void)height;
    (void)states;
}

static void toplevel_close(void *data, struct xdg_toplevel *toplevel) {
    (void)toplevel;
    struct wl_host *h = data;
    h->configured = -1;
}

static const struct xdg_toplevel_listener toplevel_listener = {toplevel_configure, toplevel_close};

static int wl_fail(const char *what, MigoResult result) {
    fprintf(stderr, "[wl-host] %s failed: %d\n", what, (int)result);
    return 1;
}

/* Declared in main.c; shared so both hosts report the engine identically. */
MigoResult MIGO_CALL wl_dispatch_inline(void *dispatcher_context, MigoTaskFn task,
                                        void *task_context);
void MIGO_CALL wl_on_ready(void *user_data, MigoSession *session);
void MIGO_CALL wl_on_error(void *user_data, MigoSession *session, const MigoError *error);

int run_wayland_host(const char *files_dir, const char *content_id, int seconds);

int run_wayland_host(const char *files_dir, const char *content_id, int seconds) {
    struct wl_host h;
    memset(&h, 0, sizeof h);

    h.display = wl_display_connect(NULL);
    if (h.display == NULL) {
        fprintf(stderr, "[wl-host] wl_display_connect failed (WAYLAND_DISPLAY=%s)\n",
                getenv("WAYLAND_DISPLAY") ? getenv("WAYLAND_DISPLAY") : "(unset)");
        return 1;
    }

    struct wl_registry *registry = wl_display_get_registry(h.display);
    wl_registry_add_listener(registry, &registry_listener, &h);
    wl_display_roundtrip(h.display);
    if (h.compositor == NULL || h.wm_base == NULL) {
        fprintf(stderr, "[wl-host] compositor lacks wl_compositor or xdg_wm_base\n");
        return 1;
    }
    xdg_wm_base_add_listener(h.wm_base, &wm_base_listener, &h);

    h.surface = wl_compositor_create_surface(h.compositor);
    h.xdg_surface = xdg_wm_base_get_xdg_surface(h.wm_base, h.surface);
    xdg_surface_add_listener(h.xdg_surface, &xdg_surface_listener, &h);
    h.toplevel = xdg_surface_get_toplevel(h.xdg_surface);
    xdg_toplevel_add_listener(h.toplevel, &toplevel_listener, &h);
    xdg_toplevel_set_title(h.toplevel, "migo-c-host-wayland");

    /* The bufferless commit that asks for a configure. Attaching before the
     * configure arrives hands EGL a surface the compositor has not agreed to
     * show, and nothing is ever presented. */
    wl_surface_commit(h.surface);
    while (!h.configured) {
        if (wl_display_dispatch(h.display) < 0) {
            fprintf(stderr, "[wl-host] dispatch failed before the first configure\n");
            return 1;
        }
    }

    char cache_dir[512];
    char code_cache_dir[512];
    snprintf(cache_dir, sizeof cache_dir, "%s/../cache", files_dir);
    snprintf(code_cache_dir, sizeof code_cache_dir, "%s/../code-cache", files_dir);

    MigoEngineConfig engine_config;
    memset(&engine_config, 0, sizeof engine_config);
    engine_config.struct_size = (uint32_t)sizeof engine_config;
    engine_config.abi_version = MIGO_ABI_VERSION_CURRENT;
    engine_config.flags = MIGO_ENGINE_FLAG_ALLOW_UNSIGNED_CONTENT;
    engine_config.files_dir_utf8 = files_dir;
    engine_config.cache_dir_utf8 = cache_dir;
    engine_config.code_cache_dir_utf8 = code_cache_dir;

    MigoCapabilities caps;
    memset(&caps, 0, sizeof caps);
    caps.struct_size = (uint32_t)sizeof caps;
    caps.abi_version = MIGO_ABI_VERSION_CURRENT;
    MigoResult result = migo_query_capabilities(&caps);
    if (result != MIGO_OK) return wl_fail("migo_query_capabilities", result);
    if ((caps.platform_kinds & (UINT64_C(1) << MIGO_PLATFORM_WAYLAND_SURFACE)) == 0) {
        fprintf(stderr, "[wl-host] this build cannot attach a Wayland surface\n");
        return 1;
    }

    MigoEngine *engine = NULL;
    result = migo_engine_create(&engine_config, &engine);
    if (result != MIGO_OK) return wl_fail("migo_engine_create", result);

    MigoSessionConfig session_config;
    memset(&session_config, 0, sizeof session_config);
    session_config.struct_size = (uint32_t)sizeof session_config;
    session_config.abi_version = MIGO_ABI_VERSION_CURRENT;
    session_config.flags = MIGO_SESSION_FLAG_NONE;

    MigoSession *session = NULL;
    result = migo_session_create(engine, &session_config, &session);
    if (result != MIGO_OK) return wl_fail("migo_session_create", result);

    MigoHostCallbacks callbacks;
    memset(&callbacks, 0, sizeof callbacks);
    callbacks.struct_size = (uint32_t)sizeof callbacks;
    callbacks.abi_version = MIGO_ABI_VERSION_CURRENT;
    callbacks.dispatch = wl_dispatch_inline;
    callbacks.on_ready = wl_on_ready;
    callbacks.on_error = wl_on_error;
    result = migo_session_set_host_callbacks(session, &callbacks);
    if (result != MIGO_OK) return wl_fail("migo_session_set_host_callbacks", result);

    MigoWaylandSurfaceDescriptor wayland;
    memset(&wayland, 0, sizeof wayland);
    wayland.struct_size = (uint32_t)sizeof wayland;
    wayland.abi_version = MIGO_ABI_VERSION_CURRENT;
    wayland.platform_kind = MIGO_PLATFORM_WAYLAND_SURFACE;
    wayland.flags = MIGO_PLATFORM_DESCRIPTOR_FLAG_NONE;
    wayland.display = h.display;
    wayland.surface = h.surface;

    MigoSurfaceDescriptor surface;
    memset(&surface, 0, sizeof surface);
    surface.struct_size = (uint32_t)sizeof surface;
    surface.abi_version = MIGO_ABI_VERSION_CURRENT;
    surface.generation = 1;
    surface.platform_kind = MIGO_PLATFORM_WAYLAND_SURFACE;
    surface.flags = MIGO_SURFACE_DESCRIPTOR_FLAG_NONE;
    surface.width_pixels = WINDOW_WIDTH;
    surface.height_pixels = WINDOW_HEIGHT;
    surface.scale_factor = 1.0f;
    surface.color_space = MIGO_COLOR_SPACE_SRGB;
    surface.alpha_mode = MIGO_ALPHA_MODE_OPAQUE;
    surface.preferred_presentation_mode = MIGO_PRESENTATION_MODE_DEFAULT;
    surface.capability_flags = MIGO_SURFACE_CAPABILITY_NONE;
    surface.platform_descriptor_size = (uint32_t)sizeof wayland;
    surface.platform_descriptor = &wayland;

    MigoSurfaceAttachment *attachment = NULL;
    result = migo_session_attach_surface(session, &surface, &attachment);
    if (result != MIGO_OK) return wl_fail("migo_session_attach_surface", result);

    MigoContentDescriptor content;
    memset(&content, 0, sizeof content);
    content.struct_size = (uint32_t)sizeof content;
    content.abi_version = MIGO_ABI_VERSION_CURRENT;
    content.flags = MIGO_CONTENT_FLAG_NONE;
    content.content_id_utf8 = content_id;
    content.entry_utf8 = "game.js";
    result = migo_session_load_content(session, &content);
    if (result != MIGO_OK) return wl_fail("migo_session_load_content", result);

    printf("[wl-host] running '%s' for %ds on a Wayland surface\n", content_id, seconds);
    fflush(stdout);

    /* The host owns dispatch. Migo renders on its own thread and never touches
     * this connection. */
    struct timespec started;
    clock_gettime(CLOCK_MONOTONIC, &started);
    for (;;) {
        struct timespec now;
        clock_gettime(CLOCK_MONOTONIC, &now);
        if (now.tv_sec - started.tv_sec >= seconds) break;
        if (h.configured < 0) {
            printf("[wl-host] window close requested\n");
            break;
        }
        /* Read the socket, do not merely drain what is already queued.
         *
         * eglSwapBuffers on Wayland waits for the compositor to release a
         * buffer, and that release arrives as an event nobody reads unless the
         * host reads it. Calling only wl_display_dispatch_pending here looks
         * like a working loop and renders exactly one frame, then blocks the
         * render thread forever -- with no error anywhere, because nothing has
         * failed. This is the prepare/read/dispatch dance that fixes it, with a
         * poll so the loop still notices its own deadline. */
        while (wl_display_prepare_read(h.display) != 0) {
            wl_display_dispatch_pending(h.display);
        }
        wl_display_flush(h.display);

        struct pollfd pfd = {wl_display_get_fd(h.display), POLLIN, 0};
        if (poll(&pfd, 1, 16) > 0 && (pfd.revents & POLLIN)) {
            wl_display_read_events(h.display);
        } else {
            wl_display_cancel_read(h.display);
        }
        wl_display_dispatch_pending(h.display);
    }

    result = migo_surface_detach(attachment);
    if (result != MIGO_OK) return wl_fail("migo_surface_detach", result);
    result = migo_session_destroy(session);
    if (result != MIGO_OK) return wl_fail("migo_session_destroy", result);
    result = migo_engine_destroy(engine);
    if (result != MIGO_OK) return wl_fail("migo_engine_destroy", result);

    xdg_toplevel_destroy(h.toplevel);
    xdg_surface_destroy(h.xdg_surface);
    wl_surface_destroy(h.surface);
    wl_display_disconnect(h.display);
    printf("[wl-host] done\n");
    return 0;
}
