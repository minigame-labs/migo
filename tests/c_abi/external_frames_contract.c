#include <migo/external_frames.h>

#include <stddef.h>
#include <stdint.h>

/*
 * One layout on both pointer widths, not two that happen to agree. The record
 * holds no pointer and places its 64-bit member before the 32-bit ones, so
 * these offsets are outside the `#if UINTPTR_MAX` split every other platform
 * record needs -- and being outside it is the assertion. A field reordered into
 * the 32-bit run would still be 32 bytes on LP64 and would move on ILP32.
 */
_Static_assert(sizeof(MigoFrameIngressOutcome) == 32,
               "ingress outcome is 32 bytes on every target");
_Static_assert(offsetof(MigoFrameIngressOutcome, struct_size) == 0,
               "size prefix");
_Static_assert(offsetof(MigoFrameIngressOutcome, abi_version) == 4,
               "ABI prefix");
_Static_assert(offsetof(MigoFrameIngressOutcome, accepted_sequence) == 8,
               "the 64-bit member stays ahead of the 32-bit run");
_Static_assert(offsetof(MigoFrameIngressOutcome, decision) == 16,
               "decision offset");
_Static_assert(offsetof(MigoFrameIngressOutcome, remaining_credits) == 20,
               "remaining_credits offset");
_Static_assert(offsetof(MigoFrameIngressOutcome, wire_error_code) == 24,
               "wire_error_code offset");
_Static_assert(offsetof(MigoFrameIngressOutcome, reserved0) == 28,
               "reserved0 offset");

/*
 * Four distinct decisions. A host branches on these, and two that collide would
 * merge two different required behaviours: WOULD_BLOCK means wait and resend,
 * REJECTED means never resend these bytes.
 */
_Static_assert(MIGO_FRAME_INGRESS_ACCEPTED != MIGO_FRAME_INGRESS_WOULD_BLOCK, "1 != 2");
_Static_assert(MIGO_FRAME_INGRESS_ACCEPTED != MIGO_FRAME_INGRESS_REJECTED, "1 != 3");
_Static_assert(MIGO_FRAME_INGRESS_ACCEPTED != MIGO_FRAME_INGRESS_GENERATION_LOST, "1 != 4");
_Static_assert(MIGO_FRAME_INGRESS_WOULD_BLOCK != MIGO_FRAME_INGRESS_REJECTED, "2 != 3");
_Static_assert(MIGO_FRAME_INGRESS_WOULD_BLOCK != MIGO_FRAME_INGRESS_GENERATION_LOST, "2 != 4");
_Static_assert(MIGO_FRAME_INGRESS_REJECTED != MIGO_FRAME_INGRESS_GENERATION_LOST, "3 != 4");

/* Zero is not a decision: a zeroed record must not read as a valid answer. */
_Static_assert(MIGO_FRAME_INGRESS_ACCEPTED != UINT32_C(0), "zero is not ACCEPTED");
_Static_assert(MIGO_FRAME_INGRESS_WOULD_BLOCK != UINT32_C(0), "zero is not WOULD_BLOCK");
_Static_assert(MIGO_FRAME_INGRESS_REJECTED != UINT32_C(0), "zero is not REJECTED");
_Static_assert(MIGO_FRAME_INGRESS_GENERATION_LOST != UINT32_C(0), "zero is not GENERATION_LOST");

int migo_external_frames_c_contract(void) {
    MigoFrameIngressOutcome outcome = {0};
    outcome.struct_size = (uint32_t)sizeof outcome;
    outcome.decision = MIGO_FRAME_INGRESS_ACCEPTED;
    return (int)(outcome.struct_size + outcome.decision);
}
