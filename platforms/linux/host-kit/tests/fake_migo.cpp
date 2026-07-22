#include "fake_migo.hpp"

#include <cstring>

namespace {

fake_migo::Calls g_calls;
MigoSurfaceDescriptor g_surface{};
MigoSurfaceMetrics g_metrics{};
MigoSurfaceReleaseStatus g_release_query{};
MigoX11WindowDescriptor g_x11{};
MigoWaylandSurfaceDescriptor g_wayland{};
MigoResult g_attach_result = MIGO_OK;
bool g_attach_returns_null = false;
bool g_attach_writes_handle_on_error = false;
MigoResult g_update_result = MIGO_OK;
MigoResult g_begin_detach_result = MIGO_OK;
bool g_begin_detach_returns_null = false;
bool g_begin_detach_writes_handle_on_error = false;
MigoResult g_query_result = MIGO_OK;
MigoSurfaceReleaseStatus g_release_status{};
MigoResult g_destroy_result = MIGO_OK;

constexpr uintptr_t kSessionToken = 0x1010;
constexpr uintptr_t kAttachmentToken = 0x2020;
constexpr uintptr_t kReleaseToken = 0x3030;

}  // namespace

namespace fake_migo {

void reset() noexcept {
    g_calls = {};
    g_surface = {};
    g_metrics = {};
    g_release_query = {};
    g_x11 = {};
    g_wayland = {};
    g_attach_result = MIGO_OK;
    g_attach_returns_null = false;
    g_attach_writes_handle_on_error = false;
    g_update_result = MIGO_OK;
    g_begin_detach_result = MIGO_OK;
    g_begin_detach_returns_null = false;
    g_begin_detach_writes_handle_on_error = false;
    g_query_result = MIGO_OK;
    g_release_status = {};
    g_release_status.struct_size = sizeof(g_release_status);
    g_release_status.abi_version = MIGO_ABI_VERSION_CURRENT;
    g_release_status.state = MIGO_SURFACE_RELEASE_PENDING;
    g_destroy_result = MIGO_OK;
}

MigoSession *session() noexcept {
    return reinterpret_cast<MigoSession *>(kSessionToken);
}

MigoSurfaceAttachment *attachment() noexcept {
    return reinterpret_cast<MigoSurfaceAttachment *>(kAttachmentToken);
}

MigoSurfaceRelease *release() noexcept {
    return reinterpret_cast<MigoSurfaceRelease *>(kReleaseToken);
}

const Calls &calls() noexcept { return g_calls; }
const MigoSurfaceDescriptor &last_surface() noexcept { return g_surface; }
const MigoSurfaceMetrics &last_metrics() noexcept { return g_metrics; }
const MigoSurfaceReleaseStatus &last_release_query() noexcept { return g_release_query; }
const MigoX11WindowDescriptor &last_x11() noexcept { return g_x11; }
const MigoWaylandSurfaceDescriptor &last_wayland() noexcept { return g_wayland; }

void set_attach_result(MigoResult result) noexcept { g_attach_result = result; }
void set_attach_returns_null(bool returns_null) noexcept { g_attach_returns_null = returns_null; }
void set_attach_writes_handle_on_error(bool writes_handle) noexcept {
    g_attach_writes_handle_on_error = writes_handle;
}
void set_update_result(MigoResult result) noexcept { g_update_result = result; }
void set_begin_detach_result(MigoResult result) noexcept { g_begin_detach_result = result; }
void set_begin_detach_returns_null(bool returns_null) noexcept {
    g_begin_detach_returns_null = returns_null;
}
void set_begin_detach_writes_handle_on_error(bool writes_handle) noexcept {
    g_begin_detach_writes_handle_on_error = writes_handle;
}
void set_query_result(MigoResult result) noexcept { g_query_result = result; }
void set_release_status(uint64_t generation, MigoSurfaceReleaseState state) noexcept {
    g_release_status.generation = generation;
    g_release_status.state = state;
}
void set_release_status_metadata(uint32_t struct_size, uint32_t abi_version,
                                 uint32_t reserved0) noexcept {
    g_release_status.struct_size = struct_size;
    g_release_status.abi_version = abi_version;
    g_release_status.reserved0 = reserved0;
}
void set_destroy_result(MigoResult result) noexcept { g_destroy_result = result; }

}  // namespace fake_migo

extern "C" MigoResult MIGO_CALL
migo_session_attach_surface(MigoSession *session, const MigoSurfaceDescriptor *descriptor,
                            MigoSurfaceAttachment **out_attachment) {
    ++g_calls.attach;
    if (out_attachment != nullptr) *out_attachment = nullptr;
    if (session == nullptr || descriptor == nullptr || out_attachment == nullptr) {
        return MIGO_ERROR_INVALID_ARGUMENT;
    }
    g_surface = *descriptor;
    if (descriptor->platform_descriptor != nullptr) {
        if (descriptor->platform_kind == MIGO_PLATFORM_X11_WINDOW) {
            std::memcpy(&g_x11, descriptor->platform_descriptor, sizeof(g_x11));
            g_surface.platform_descriptor = &g_x11;
        } else if (descriptor->platform_kind == MIGO_PLATFORM_WAYLAND_SURFACE) {
            std::memcpy(&g_wayland, descriptor->platform_descriptor, sizeof(g_wayland));
            g_surface.platform_descriptor = &g_wayland;
        }
    }
    if ((g_attach_result == MIGO_OK && !g_attach_returns_null) ||
        (g_attach_result != MIGO_OK && g_attach_writes_handle_on_error)) {
        *out_attachment = fake_migo::attachment();
    }
    return g_attach_result;
}

extern "C" MigoResult MIGO_CALL
migo_surface_update(MigoSurfaceAttachment *attachment, const MigoSurfaceMetrics *metrics) {
    ++g_calls.update;
    if (attachment == nullptr || metrics == nullptr) return MIGO_ERROR_INVALID_ARGUMENT;
    g_metrics = *metrics;
    return g_update_result;
}

extern "C" MigoResult MIGO_CALL
migo_surface_begin_detach(MigoSurfaceAttachment *attachment, MigoSurfaceRelease **out_release) {
    ++g_calls.begin_detach;
    if (out_release != nullptr) *out_release = nullptr;
    if (attachment == nullptr || out_release == nullptr) return MIGO_ERROR_INVALID_ARGUMENT;
    if ((g_begin_detach_result == MIGO_OK && !g_begin_detach_returns_null) ||
        (g_begin_detach_result != MIGO_OK && g_begin_detach_writes_handle_on_error)) {
        *out_release = fake_migo::release();
    }
    return g_begin_detach_result;
}

extern "C" MigoResult MIGO_CALL
migo_surface_release_query(const MigoSurfaceRelease *release,
                           MigoSurfaceReleaseStatus *out_status) {
    ++g_calls.query;
    if (release == nullptr || out_status == nullptr) return MIGO_ERROR_INVALID_ARGUMENT;
    g_release_query = *out_status;
    if (g_query_result == MIGO_OK) *out_status = g_release_status;
    return g_query_result;
}

extern "C" MigoResult MIGO_CALL migo_surface_release_destroy(MigoSurfaceRelease *release) {
    ++g_calls.destroy_release;
    if (release == nullptr) return MIGO_ERROR_INVALID_ARGUMENT;
    return g_destroy_result;
}
