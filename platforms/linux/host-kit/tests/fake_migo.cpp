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
MigoResult g_input_result = MIGO_OK;
std::vector<fake_migo::TouchRecord> g_touches;
std::vector<fake_migo::PointerRecord> g_pointers;
std::vector<fake_migo::WheelRecord> g_wheels;
std::vector<fake_migo::KeyRecord> g_keys;
std::vector<fake_migo::CompositionRecord> g_compositions;
std::vector<uint8_t> g_focus_changes;
std::vector<int64_t> g_vsyncs;

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
    g_input_result = MIGO_OK;
    g_touches.clear();
    g_pointers.clear();
    g_wheels.clear();
    g_keys.clear();
    g_compositions.clear();
    g_focus_changes.clear();
    g_vsyncs.clear();
    // Reserved, not merely cleared: the allocation test measures the delivery
    // path by differencing a delivered burst against an undelivered one, and a
    // vector growing here would land in that difference and be read as the
    // adapter allocating per event. `clear()` keeps capacity, so this only ever
    // allocates once per process.
    constexpr std::size_t kRecordCapacity = 1024;
    g_touches.reserve(kRecordCapacity);
    g_pointers.reserve(kRecordCapacity);
    g_wheels.reserve(kRecordCapacity);
    g_keys.reserve(kRecordCapacity);
    g_compositions.reserve(kRecordCapacity);
    g_focus_changes.reserve(kRecordCapacity);
    g_vsyncs.reserve(kRecordCapacity);
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
void set_input_result(MigoResult result) noexcept { g_input_result = result; }

const std::vector<TouchRecord> &touches() noexcept { return g_touches; }
const std::vector<PointerRecord> &pointers() noexcept { return g_pointers; }
const std::vector<WheelRecord> &wheels() noexcept { return g_wheels; }
const std::vector<KeyRecord> &keys() noexcept { return g_keys; }
const std::vector<CompositionRecord> &compositions() noexcept { return g_compositions; }
const std::vector<uint8_t> &focus_changes() noexcept { return g_focus_changes; }
const std::vector<int64_t> &vsyncs() noexcept { return g_vsyncs; }

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

extern "C" MigoResult MIGO_CALL migo_session_send_touch(MigoSession *session,
                                                        const MigoTouchEvent *event) {
    ++g_calls.touch;
    if (session == nullptr || event == nullptr) return MIGO_ERROR_INVALID_ARGUMENT;
    if (event->struct_size != sizeof(*event)) return MIGO_ERROR_INVALID_ARGUMENT;
    if (event->points == nullptr || event->point_count == 0) return MIGO_ERROR_INVALID_ARGUMENT;
    fake_migo::TouchRecord record;
    record.type = event->type;
    record.timestamp_ms = event->timestamp_ms;
    record.points.assign(event->points, event->points + event->point_count);
    g_touches.push_back(std::move(record));
    return g_input_result;
}

extern "C" MigoResult MIGO_CALL migo_session_send_pointer_event(MigoSession *session,
                                                                const MigoPointerEvent *event) {
    ++g_calls.pointer;
    if (session == nullptr || event == nullptr) return MIGO_ERROR_INVALID_ARGUMENT;
    if (event->struct_size != sizeof(*event)) return MIGO_ERROR_INVALID_ARGUMENT;
    g_pointers.push_back({event->event_type, event->button, event->x, event->y,
                          event->timestamp_ms});
    return g_input_result;
}

extern "C" MigoResult MIGO_CALL migo_session_send_wheel_event(MigoSession *session,
                                                              const MigoWheelEvent *event) {
    ++g_calls.wheel;
    if (session == nullptr || event == nullptr) return MIGO_ERROR_INVALID_ARGUMENT;
    if (event->struct_size != sizeof(*event)) return MIGO_ERROR_INVALID_ARGUMENT;
    if (event->reserved0 != 0) return MIGO_ERROR_INVALID_ARGUMENT;
    g_wheels.push_back({event->delta_x, event->delta_y, event->delta_z, event->delta_mode,
                        event->timestamp_ms});
    return g_input_result;
}

extern "C" MigoResult MIGO_CALL migo_session_send_key_event(MigoSession *session,
                                                            const MigoKeyEvent *event) {
    ++g_calls.key;
    if (session == nullptr || event == nullptr) return MIGO_ERROR_INVALID_ARGUMENT;
    if (event->struct_size != sizeof(*event)) return MIGO_ERROR_INVALID_ARGUMENT;
    if (event->reserved0 != 0) return MIGO_ERROR_INVALID_ARGUMENT;
    // The real library rejects an empty code, so the fake must too: a bridge
    // that could not identify the physical key must say "Unidentified" rather
    // than send nothing.
    if (event->code_utf8 == nullptr || event->code_length == 0) {
        return MIGO_ERROR_INVALID_ARGUMENT;
    }
    fake_migo::KeyRecord record;
    record.event_type = event->event_type;
    if (event->key_utf8 != nullptr) record.key.assign(event->key_utf8, event->key_length);
    record.code.assign(event->code_utf8, event->code_length);
    record.modifiers = event->modifiers;
    record.flags = event->flags;
    record.timestamp_ms = event->timestamp_ms;
    g_keys.push_back(std::move(record));
    return g_input_result;
}

extern "C" MigoResult MIGO_CALL
migo_session_send_composition_event(MigoSession *session, const MigoCompositionEvent *event) {
    ++g_calls.composition;
    if (session == nullptr || event == nullptr) return MIGO_ERROR_INVALID_ARGUMENT;
    if (event->struct_size != sizeof(*event)) return MIGO_ERROR_INVALID_ARGUMENT;
    fake_migo::CompositionRecord record;
    record.event_type = event->event_type;
    if (event->data_utf8 != nullptr) record.data.assign(event->data_utf8, event->data_length);
    g_compositions.push_back(std::move(record));
    return g_input_result;
}

extern "C" MigoResult MIGO_CALL migo_session_set_focus(MigoSession *session, uint8_t focused) {
    ++g_calls.focus;
    if (session == nullptr) return MIGO_ERROR_INVALID_ARGUMENT;
    g_focus_changes.push_back(focused);
    return g_input_result;
}

extern "C" MigoResult MIGO_CALL migo_session_notify_vsync(MigoSession *session,
                                                          int64_t frame_time_nanos) {
    ++g_calls.vsync;
    if (session == nullptr) return MIGO_ERROR_INVALID_ARGUMENT;
    g_vsyncs.push_back(frame_time_nanos);
    return g_input_result;
}
