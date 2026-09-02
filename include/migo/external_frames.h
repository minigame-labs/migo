#ifndef MIGO_EXTERNAL_FRAMES_H_
#define MIGO_EXTERNAL_FRAMES_H_

#include <migo/types.h>

/*
 * The record a host reads back after offering one frame produced outside this
 * process.
 *
 * On iOS the producer is a Worker inside WebKit's WebContent process: content
 * JavaScript runs there to get the system's JIT, encodes a frame's drawing work
 * into one bounded binary packet, and sends it here, where Migo renders it. The
 * Swift transport that carries those bytes never touches a Rust type and never
 * links a JavaScript engine; this record is the whole of what it sees coming
 * back.
 *
 * DECLARATIONS ONLY, ON PURPOSE. There is no submit function here yet. The
 * layout is pinned now because the Swift side is written against it and because
 * a layout agreed late is a layout agreed twice; the entry point lands with the
 * implementation behind it. An exported symbol that always fails is the shape
 * that shipped a Windows SDK which loaded, resolved every entry point, and
 * could attach nothing.
 */

typedef uint32_t MigoFrameIngressDecision;

/* Taken. A credit is consumed until the renderer reports completion. */
#define MIGO_FRAME_INGRESS_ACCEPTED UINT32_C(1)
/*
 * Legal, but no credit is available. The producer must wait. It must not drop
 * the packet: a frame may carry state or resource changes that a later frame
 * depends on, so "skip it" is not a correct answer on either side.
 */
#define MIGO_FRAME_INGRESS_WOULD_BLOCK UINT32_C(2)
/*
 * Malformed, or not addressed to this session. Costs no credit -- otherwise a
 * producer sending garbage would exhaust its own window and stall, which reads
 * on a device as a hang rather than as bad input. The producer must not resend
 * the same bytes.
 */
#define MIGO_FRAME_INGRESS_REJECTED UINT32_C(3)
/*
 * Correct bytes for a runtime generation that no longer exists -- the WebContent
 * process was replaced, or the session reloaded. Distinct from REJECTED because
 * nobody did anything wrong and no retry helps; reporting it as an error sends
 * whoever reads the telemetry looking for a bug that is not there.
 */
#define MIGO_FRAME_INGRESS_GENERATION_LOST UINT32_C(4)

/*
 * Library-written, so it grows append-only: a caller compiled against an
 * earlier version must keep reading the same bytes at the same offsets.
 *
 * accepted_sequence precedes the 32-bit fields deliberately. Placing the 64-bit
 * member first makes the record 32 bytes with no interior padding on both LP64
 * and ILP32, so there is one layout rather than two that happen to agree.
 */
typedef struct MigoFrameIngressOutcome {
    uint32_t struct_size;
    uint32_t abi_version;
    /* Non-zero only for ACCEPTED. */
    uint64_t accepted_sequence;
    MigoFrameIngressDecision decision;
    uint32_t remaining_credits;
    /*
     * Non-zero only for REJECTED. Stable across releases so production
     * telemetry can tell "the producer sent a short packet" apart from "the
     * producer sent another session's packet". Envelope failures are numbered
     * from 1; identity and ordering failures from 1001, so one field carries
     * either without ambiguity.
     */
    uint32_t wire_error_code;
    uint32_t reserved0;
} MigoFrameIngressOutcome;

#endif /* MIGO_EXTERNAL_FRAMES_H_ */
