//! The ingress outcome record: its consistency rules, and its agreement with
//! the wire reader that produces the values.

use migo_capi_abi::{
    MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_INVALID_ARGUMENT, MIGO_OK,
    external_frames::{
        MIGO_FRAME_INGRESS_ACCEPTED, MIGO_FRAME_INGRESS_GENERATION_LOST,
        MIGO_FRAME_INGRESS_REJECTED, MIGO_FRAME_INGRESS_WOULD_BLOCK, MigoFrameIngressOutcome,
        write_frame_ingress_outcome,
    },
};

fn write(decision: u32, credits: u32, sequence: u64, error: u32) -> (i32, MigoFrameIngressOutcome) {
    let mut out = MigoFrameIngressOutcome {
        header: migo_capi_abi::VersionedHeader {
            struct_size: size_of::<MigoFrameIngressOutcome>() as u32,
            abi_version: MIGO_ABI_VERSION_CURRENT,
        },
        accepted_sequence: 0,
        decision: 0,
        remaining_credits: 0,
        wire_error_code: 0,
        reserved0: 0,
    };
    // SAFETY: `out` is a live, correctly sized, correctly versioned record.
    let result =
        unsafe { write_frame_ingress_outcome(&mut out, decision, credits, sequence, error) };
    (result, out)
}

#[test]
fn a_well_formed_outcome_round_trips() {
    let (result, out) = write(MIGO_FRAME_INGRESS_ACCEPTED, 1, 42, 0);
    assert_eq!(result, MIGO_OK);
    assert_eq!(out.decision, MIGO_FRAME_INGRESS_ACCEPTED);
    assert_eq!(out.accepted_sequence, 42);
    assert_eq!(out.remaining_credits, 1);
    assert_eq!(out.wire_error_code, 0);
    assert_eq!(out.reserved0, 0);
    assert_eq!(
        out.header.struct_size,
        size_of::<MigoFrameIngressOutcome>() as u32
    );
}

#[test]
fn a_sequence_number_means_the_packet_was_taken() {
    // Only ACCEPTED takes one. The natural bug is an early return that forgets
    // to clear the field, leaving a rejection that names a frame it did not
    // accept -- and the host's own bookkeeping keys on exactly that field.
    assert_eq!(
        write(MIGO_FRAME_INGRESS_ACCEPTED, 1, 0, 0).0,
        MIGO_ERROR_INVALID_ARGUMENT,
        "ACCEPTED without a sequence",
    );
    for decision in [
        MIGO_FRAME_INGRESS_WOULD_BLOCK,
        MIGO_FRAME_INGRESS_GENERATION_LOST,
    ] {
        assert_eq!(
            write(decision, 0, 7, 0).0,
            MIGO_ERROR_INVALID_ARGUMENT,
            "decision {decision} carried a sequence number",
        );
    }
    assert_eq!(
        write(MIGO_FRAME_INGRESS_REJECTED, 1, 7, 5).0,
        MIGO_ERROR_INVALID_ARGUMENT,
        "REJECTED carried a sequence number",
    );
}

#[test]
fn an_error_code_means_the_bytes_were_refused() {
    assert_eq!(
        write(MIGO_FRAME_INGRESS_REJECTED, 1, 0, 0).0,
        MIGO_ERROR_INVALID_ARGUMENT,
        "REJECTED without a reason",
    );
    // GENERATION_LOST is not a fault and WOULD_BLOCK is not a verdict on the
    // bytes, so neither may carry one.
    assert_eq!(
        write(MIGO_FRAME_INGRESS_GENERATION_LOST, 1, 0, 14).0,
        MIGO_ERROR_INVALID_ARGUMENT,
    );
    assert_eq!(
        write(MIGO_FRAME_INGRESS_WOULD_BLOCK, 0, 0, 14).0,
        MIGO_ERROR_INVALID_ARGUMENT,
    );
}

#[test]
fn would_block_is_exactly_the_no_credit_answer() {
    assert_eq!(write(MIGO_FRAME_INGRESS_WOULD_BLOCK, 0, 0, 0).0, MIGO_OK);
    // Advertising credit alongside a refusal would tell the producer to retry
    // straight back into the same refusal.
    assert_eq!(
        write(MIGO_FRAME_INGRESS_WOULD_BLOCK, 1, 0, 0).0,
        MIGO_ERROR_INVALID_ARGUMENT,
    );
}

#[test]
fn an_unrecognised_decision_is_rejected_including_zero() {
    for decision in [0u32, 5, u32::MAX] {
        assert_eq!(
            write(decision, 0, 0, 0).0,
            MIGO_ERROR_INVALID_ARGUMENT,
            "decision {decision} must not be writable",
        );
    }
}

/// The four numbers here and the four in the wire reader are one decision. This
/// is the only place that can see both, so it is the only place the drift can
/// be caught -- `frame-wire` stays out of the C ABI crate's shipping closure,
/// and the ABI crate stays out of the wire reader's.
#[test]
fn the_abi_and_the_wire_reader_agree_on_every_decision() {
    use frame_wire::IngressDecision;
    assert_eq!(
        IngressDecision::Accepted as u32,
        MIGO_FRAME_INGRESS_ACCEPTED
    );
    assert_eq!(
        IngressDecision::WouldBlock as u32,
        MIGO_FRAME_INGRESS_WOULD_BLOCK
    );
    assert_eq!(
        IngressDecision::Rejected as u32,
        MIGO_FRAME_INGRESS_REJECTED
    );
    assert_eq!(
        IngressDecision::GenerationLost as u32,
        MIGO_FRAME_INGRESS_GENERATION_LOST
    );
}

/// Every wire rejection reason must be expressible in the field that carries
/// it, and must be distinguishable from "no error".
///
/// The lists come from `frame-wire`, which proves them complete against its own
/// source. Writing them out here instead would make this test cover every
/// reason as of whenever someone last updated it -- and a reason added later
/// would be exactly the one that could not be reported.
#[test]
fn every_wire_error_code_is_non_zero_and_distinct_from_the_ingress_range() {
    use frame_wire::{WireError, ingress::INGRESS_ERROR_BASE};

    assert!(
        WireError::ALL.len() >= 20,
        "the envelope has more rejection reasons than this: {}",
        WireError::ALL.len()
    );

    for error in WireError::ALL {
        let code = error.code();
        assert_ne!(code, 0, "{error:?} would read as 'no error'");
        assert!(
            code < INGRESS_ERROR_BASE,
            "{error:?} is {code}, inside the range reserved for identity and \
             ordering failures",
        );
        // And each one must survive the write path it is destined for.
        let (result, out) = write(MIGO_FRAME_INGRESS_REJECTED, 2, 0, code);
        assert_eq!(result, MIGO_OK, "{error:?} could not be reported");
        assert_eq!(out.wire_error_code, code);
    }

    for code in frame_wire::ingress::INGRESS_ERROR_CODES {
        assert!(
            *code >= INGRESS_ERROR_BASE,
            "identity failures start at {INGRESS_ERROR_BASE}, found {code}"
        );
        let (result, out) = write(MIGO_FRAME_INGRESS_REJECTED, 2, 0, *code);
        assert_eq!(result, MIGO_OK, "code {code} could not be reported");
        assert_eq!(out.wire_error_code, *code);
    }
}
