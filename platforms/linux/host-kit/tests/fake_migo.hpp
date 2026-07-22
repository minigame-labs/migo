#ifndef MIGO_LINUX_HOST_KIT_TESTS_FAKE_MIGO_HPP_
#define MIGO_LINUX_HOST_KIT_TESTS_FAKE_MIGO_HPP_

#include <migo/migo.h>
#include <migo/platform/wayland.h>
#include <migo/platform/x11.h>

namespace fake_migo {

struct Calls {
    int attach = 0;
    int update = 0;
    int begin_detach = 0;
    int query = 0;
    int destroy_release = 0;
};

void reset() noexcept;

MigoSession *session() noexcept;
MigoSurfaceAttachment *attachment() noexcept;
MigoSurfaceRelease *release() noexcept;

const Calls &calls() noexcept;
const MigoSurfaceDescriptor &last_surface() noexcept;
const MigoSurfaceMetrics &last_metrics() noexcept;
const MigoSurfaceReleaseStatus &last_release_query() noexcept;
const MigoX11WindowDescriptor &last_x11() noexcept;
const MigoWaylandSurfaceDescriptor &last_wayland() noexcept;

void set_attach_result(MigoResult result) noexcept;
void set_attach_returns_null(bool returns_null) noexcept;
void set_attach_writes_handle_on_error(bool writes_handle) noexcept;
void set_update_result(MigoResult result) noexcept;
void set_begin_detach_result(MigoResult result) noexcept;
void set_begin_detach_returns_null(bool returns_null) noexcept;
void set_begin_detach_writes_handle_on_error(bool writes_handle) noexcept;
void set_query_result(MigoResult result) noexcept;
void set_release_status(uint64_t generation, MigoSurfaceReleaseState state) noexcept;
void set_release_status_metadata(uint32_t struct_size, uint32_t abi_version,
                                 uint32_t reserved0) noexcept;
void set_destroy_result(MigoResult result) noexcept;

}  // namespace fake_migo

#endif  // MIGO_LINUX_HOST_KIT_TESTS_FAKE_MIGO_HPP_
