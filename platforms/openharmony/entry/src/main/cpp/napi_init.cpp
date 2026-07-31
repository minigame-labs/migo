/*
 * OpenHarmony native host for migo.
 *
 * The engine is reached only through the public C ABI in <migo/*.h>; nothing
 * here includes an engine header. That is the same discipline the Android
 * NativeActivity host and the Linux/Win32 hosts follow, and it is what makes
 * this file a consumer of the SDK rather than part of it.
 *
 * The surface arrives from ArkUI's XComponent, whose OnSurfaceCreated callback
 * hands over an OHNativeWindow*. That pointer is exactly what
 * MigoOpenHarmonyNativeWindowDescriptor carries, so no translation is needed --
 * only ownership discipline: the host keeps its reference, the engine takes its
 * own, and the host must not destroy the window until the release observer
 * reports RELEASED.
 */

#include <ace/xcomponent/native_interface_xcomponent.h>
#include <hilog/log.h>
#include <napi/native_api.h>

#include <cstring>
#include <string>

#include <migo/migo.h>
/* migo.h is the engine/session umbrella and deliberately pulls in no platform
 * descriptor: including one would drag a platform SDK header into hosts that
 * have nothing to do with it. The typed descriptor is opted into here. */
#include <migo/platform/openharmony.h>

#define LOG_TAG "migo-host"
#define LOGI(...) OH_LOG_Print(LOG_APP, LOG_INFO, 0xF000, LOG_TAG, __VA_ARGS__)
#define LOGE(...) OH_LOG_Print(LOG_APP, LOG_ERROR, 0xF000, LOG_TAG, __VA_ARGS__)

namespace {

struct Host {
    MigoEngine *engine = nullptr;
    MigoSession *session = nullptr;
    MigoSurfaceAttachment *attachment = nullptr;
    uint64_t generation = 0;
    bool content_loaded = false;
    std::string files_dir;
    std::string cache_dir;
    std::string content_id;
};

Host g_host;

/*
 * Every user callback must be delivered through a host-owned dispatcher: the
 * engine produces these on its own worker threads, and running host code there
 * unasked is precisely what the ABI forbids. This host has no event loop of its
 * own yet, so it runs the task inline and says so -- an honest minimal
 * dispatcher rather than one that pretends to marshal.
 */
MigoResult dispatch_inline(void *dispatcher_context, MigoTaskFn task, void *task_context) {
    (void)dispatcher_context;
    if (task == nullptr) {
        return MIGO_ERROR_INVALID_ARGUMENT;
    }
    task(task_context);
    return MIGO_OK;
}

void on_ready(void *user_data, MigoSession *session) {
    (void)user_data;
    (void)session;
    LOGI("content is ready");
}

void on_error(void *user_data, MigoSession *session, const MigoError *error) {
    (void)user_data;
    (void)session;
    if (error != nullptr && error->message_utf8 != nullptr) {
        LOGE("engine error %{public}d: %{public}.*s", (int)error->code,
             (int)error->message_length, error->message_utf8);
    } else {
        LOGE("engine error with no message");
    }
}

void on_exit_requested(void *user_data, MigoSession *session) {
    (void)user_data;
    (void)session;
    LOGI("content requested exit");
}

void attach_surface(OH_NativeXComponent *component, void *window) {
    if (g_host.session == nullptr || window == nullptr || g_host.attachment != nullptr) {
        return;
    }

    uint64_t width = 0;
    uint64_t height = 0;
    if (OH_NativeXComponent_GetXComponentSize(component, window, &width, &height) != 0) {
        LOGE("OH_NativeXComponent_GetXComponentSize failed");
        return;
    }
    LOGI("surface created %{public}llu x %{public}llu", (unsigned long long)width,
         (unsigned long long)height);

    MigoOpenHarmonyNativeWindowDescriptor native;
    memset(&native, 0, sizeof native);
    native.struct_size = (uint32_t)sizeof native;
    native.abi_version = MIGO_ABI_VERSION_CURRENT;
    native.platform_kind = MIGO_PLATFORM_OPENHARMONY_NATIVE_WINDOW;
    native.flags = 0;
    /* The engine takes its own reference; this one stays ours. */
    native.native_window = window;

    MigoSurfaceDescriptor surface;
    memset(&surface, 0, sizeof surface);
    surface.struct_size = (uint32_t)sizeof surface;
    surface.abi_version = MIGO_ABI_VERSION_CURRENT;
    /* Generations are monotonic per Session and never reused, so a stale
     * attachment can be told apart from the live one. */
    surface.generation = ++g_host.generation;
    surface.platform_kind = MIGO_PLATFORM_OPENHARMONY_NATIVE_WINDOW;
    surface.flags = 0;
    surface.width_pixels = (uint32_t)width;
    surface.height_pixels = (uint32_t)height;
    /* Physical pixels per CSS pixel. A wrong value here still renders, but puts
     * every touch in the wrong place -- the failure is silent and looks like an
     * input bug rather than a configuration one. */
    surface.scale_factor = 3.0f;
    surface.color_space = MIGO_COLOR_SPACE_SRGB;
    surface.alpha_mode = MIGO_ALPHA_MODE_OPAQUE;
    surface.preferred_presentation_mode = MIGO_PRESENTATION_MODE_DEFAULT;
    surface.capability_flags = 0;
    surface.platform_descriptor_size = (uint32_t)sizeof native;
    surface.platform_descriptor = &native;

    MigoResult rc = migo_session_attach_surface(g_host.session, &surface, &g_host.attachment);
    if (rc != MIGO_OK) {
        LOGE("migo_session_attach_surface failed: %{public}d", (int)rc);
        return;
    }
    LOGI("surface attached, generation %{public}llu", (unsigned long long)surface.generation);

    if (!g_host.content_loaded && !g_host.content_id.empty()) {
        MigoContentDescriptor content;
        memset(&content, 0, sizeof content);
        content.struct_size = (uint32_t)sizeof content;
        content.abi_version = MIGO_ABI_VERSION_CURRENT;
        content.flags = 0;
        content.content_id_utf8 = g_host.content_id.c_str();
        content.entry_utf8 = nullptr;

        rc = migo_session_load_content(g_host.session, &content);
        if (rc != MIGO_OK) {
            LOGE("migo_session_load_content failed: %{public}d", (int)rc);
        } else {
            g_host.content_loaded = true;
            LOGI("loading content %{public}s", g_host.content_id.c_str());
        }
    }
}

void detach_surface() {
    if (g_host.attachment == nullptr) {
        return;
    }
    MigoSurfaceRelease *release = nullptr;
    MigoResult rc = migo_surface_begin_detach(g_host.attachment, &release);
    if (rc != MIGO_OK) {
        LOGE("migo_surface_begin_detach failed: %{public}d", (int)rc);
        return;
    }
    g_host.attachment = nullptr;

    /* The host must not free its window until this reports RELEASED: driver
     * references outlive the call and the engine cannot observe them. */
    for (;;) {
        MigoSurfaceReleaseStatus status;
        memset(&status, 0, sizeof status);
        status.struct_size = (uint32_t)sizeof status;
        status.abi_version = MIGO_ABI_VERSION_CURRENT;
        if (migo_surface_release_query(release, &status) != MIGO_OK) {
            break;
        }
        if (status.state == MIGO_SURFACE_RELEASE_RELEASED) {
            break;
        }
    }
    migo_surface_release_destroy(release);
    LOGI("surface released");
}

void OnSurfaceCreatedCB(OH_NativeXComponent *component, void *window) {
    attach_surface(component, window);
}

void OnSurfaceChangedCB(OH_NativeXComponent *component, void *window) {
    (void)component;
    (void)window;
    LOGI("surface changed");
}

void OnSurfaceDestroyedCB(OH_NativeXComponent *component, void *window) {
    (void)component;
    (void)window;
    detach_surface();
}

void DispatchTouchEventCB(OH_NativeXComponent *component, void *window) {
    (void)component;
    (void)window;
}

OH_NativeXComponent_Callback g_callbacks = {
    OnSurfaceCreatedCB,
    OnSurfaceChangedCB,
    OnSurfaceDestroyedCB,
    DispatchTouchEventCB,
};

std::string read_string_arg(napi_env env, napi_value value) {
    size_t len = 0;
    if (napi_get_value_string_utf8(env, value, nullptr, 0, &len) != napi_ok) {
        return {};
    }
    std::string out(len + 1, '\0');
    size_t written = 0;
    if (napi_get_value_string_utf8(env, value, &out[0], len + 1, &written) != napi_ok) {
        return {};
    }
    out.resize(written);
    return out;
}

/* start(filesDir: string, cacheDir: string, contentId: string): number */
napi_value Start(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value args[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, args, nullptr, nullptr);

    if (argc >= 1) g_host.files_dir = read_string_arg(env, args[0]);
    if (argc >= 2) g_host.cache_dir = read_string_arg(env, args[1]);
    if (argc >= 3) g_host.content_id = read_string_arg(env, args[2]);

    napi_value out = nullptr;

    MigoEngineConfig config;
    memset(&config, 0, sizeof config);
    config.struct_size = (uint32_t)sizeof config;
    config.abi_version = MIGO_ABI_VERSION_CURRENT;
    /* Unsigned content is a development-only allowance; the default is to
     * require a signed receipt. */
    config.flags = MIGO_ENGINE_FLAG_ALLOW_UNSIGNED_CONTENT;
    config.files_dir_utf8 = g_host.files_dir.c_str();
    config.cache_dir_utf8 = g_host.cache_dir.c_str();
    config.code_cache_dir_utf8 = g_host.cache_dir.c_str();

    MigoResult rc = migo_engine_create(&config, &g_host.engine);
    if (rc != MIGO_OK) {
        LOGE("migo_engine_create failed: %{public}d", (int)rc);
        napi_create_int32(env, (int32_t)rc, &out);
        return out;
    }

    MigoSessionConfig session_config;
    memset(&session_config, 0, sizeof session_config);
    session_config.struct_size = (uint32_t)sizeof session_config;
    session_config.abi_version = MIGO_ABI_VERSION_CURRENT;

    rc = migo_session_create(g_host.engine, &session_config, &g_host.session);
    if (rc != MIGO_OK) {
        LOGE("migo_session_create failed: %{public}d", (int)rc);
        napi_create_int32(env, (int32_t)rc, &out);
        return out;
    }

    /* Callbacks install once, before the first attach: replacing them later
     * would race queued tasks against the function pointers they captured. */
    MigoHostCallbacks callbacks;
    memset(&callbacks, 0, sizeof callbacks);
    callbacks.struct_size = (uint32_t)sizeof callbacks;
    callbacks.abi_version = MIGO_ABI_VERSION_CURRENT;
    callbacks.dispatch = dispatch_inline;
    callbacks.on_ready = on_ready;
    callbacks.on_error = on_error;
    callbacks.on_exit_requested = on_exit_requested;
    rc = migo_session_set_host_callbacks(g_host.session, &callbacks);
    if (rc != MIGO_OK) {
        LOGE("migo_session_set_host_callbacks failed: %{public}d", (int)rc);
    }

    LOGI("engine and session created");
    napi_create_int32(env, (int32_t)MIGO_OK, &out);
    return out;
}

napi_value Init(napi_env env, napi_value exports) {
    napi_property_descriptor desc[] = {
        {"start", nullptr, Start, nullptr, nullptr, nullptr, napi_default, nullptr},
    };
    napi_define_properties(env, exports, sizeof(desc) / sizeof(desc[0]), desc);

    /* Bind to the XComponent declared in the ArkTS page. Without this the
     * surface callbacks never fire and the engine is handed nothing to draw
     * on -- which presents as a silent black screen, not as an error. */
    napi_value exportInstance = nullptr;
    if (napi_get_named_property(env, exports, OH_NATIVE_XCOMPONENT_OBJ, &exportInstance) ==
        napi_ok) {
        OH_NativeXComponent *component = nullptr;
        if (napi_unwrap(env, exportInstance, reinterpret_cast<void **>(&component)) == napi_ok &&
            component != nullptr) {
            char id[OH_XCOMPONENT_ID_LEN_MAX + 1] = {};
            uint64_t id_len = OH_XCOMPONENT_ID_LEN_MAX + 1;
            if (OH_NativeXComponent_GetXComponentId(component, id, &id_len) == 0) {
                LOGI("bound XComponent id=%{public}s", id);
            }
            OH_NativeXComponent_RegisterCallback(component, &g_callbacks);
        } else {
            LOGE("napi_unwrap of the XComponent instance failed");
        }
    } else {
        LOGE("no native XComponent object on exports; is the XComponent declared?");
    }
    return exports;
}

}  // namespace

extern "C" {
static napi_module g_module = {
    .nm_version = 1,
    .nm_flags = 0,
    .nm_filename = nullptr,
    .nm_register_func = Init,
    .nm_modname = "migohost",
    .nm_priv = nullptr,
    .reserved = {nullptr},
};

__attribute__((constructor)) void RegisterMigoHostModule(void) {
    napi_module_register(&g_module);
}
}
