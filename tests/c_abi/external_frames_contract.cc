#include <migo/external_frames.h>

#include <cstddef>
#include <cstdint>
#include <type_traits>

static_assert(std::is_standard_layout<MigoFrameIngressOutcome>::value,
              "standard layout");
static_assert(std::is_trivially_copyable<MigoFrameIngressOutcome>::value,
              "trivially copyable");
static_assert(sizeof(MigoFrameIngressOutcome) == 32, "32 bytes");
static_assert(offsetof(MigoFrameIngressOutcome, accepted_sequence) == 8,
              "the 64-bit member stays ahead of the 32-bit run");
static_assert(offsetof(MigoFrameIngressOutcome, reserved0) == 28, "reserved0");

int migo_external_frames_cpp_contract() {
    MigoFrameIngressOutcome outcome{};
    outcome.decision = MIGO_FRAME_INGRESS_GENERATION_LOST;
    return static_cast<int>(outcome.decision);
}
