//! Credit accounting, identity and ordering at the ingress door.

use frame_wire::{
    FrameIngress, IngressDecision, SECTION_KIND_COMMAND_STREAM,
    builder::WireFrameBuilder,
    ingress::{
        DEFAULT_MAX_CREDITS, INGRESS_ERROR_FOREIGN_SESSION, INGRESS_ERROR_STALE_RESOURCE_EPOCH,
        INGRESS_ERROR_STALE_SEQUENCE, INGRESS_ERROR_STALE_SURFACE,
    },
};

const NONCE: u64 = 0x0123_4567_89AB_CDEF;
const GENERATION: u32 = 7;

struct Packet(Vec<u8>);

fn packet(sequence: u64) -> Packet {
    Packet(build(|builder| {
        let mut builder = builder;
        builder.sequence = sequence;
        builder
    }))
}

fn build(
    configure: impl FnOnce(WireFrameBuilder<'static>) -> WireFrameBuilder<'static>,
) -> Vec<u8> {
    static STREAM: [u8; 8] = [0; 8];
    let mut builder = WireFrameBuilder::new();
    builder.session_nonce = NONCE;
    builder.runtime_generation = GENERATION;
    configure(builder)
        .section(SECTION_KIND_COMMAND_STREAM, 2, &STREAM)
        .build()
}

fn ingress() -> FrameIngress {
    FrameIngress::new(NONCE, GENERATION)
}

#[test]
fn credits_bound_the_number_of_frames_in_flight() {
    let mut ingress = ingress();
    assert_eq!(ingress.remaining_credits(), DEFAULT_MAX_CREDITS);

    for sequence in 1..=DEFAULT_MAX_CREDITS as u64 {
        let bytes = packet(sequence);
        let (outcome, frame) = ingress.submit(&bytes.0);
        assert_eq!(outcome.decision, IngressDecision::Accepted);
        assert_eq!(outcome.accepted_sequence, sequence);
        assert!(frame.is_some());
    }
    assert_eq!(ingress.remaining_credits(), 0);

    // One past the limit waits. It is not dropped: the packet may carry state
    // or resource changes a later frame depends on, so "skip it" is not a legal
    // answer for the producer either.
    let blocked = packet(DEFAULT_MAX_CREDITS as u64 + 1);
    let (outcome, frame) = ingress.submit(&blocked.0);
    assert_eq!(outcome.decision, IngressDecision::WouldBlock);
    assert_eq!(outcome.remaining_credits, 0);
    assert!(frame.is_none());

    // And being told to wait must not consume the sequence number, or the
    // producer's retry would then be rejected as stale -- a deadlock built out
    // of two individually reasonable rules.
    ingress.complete();
    let (outcome, _) = ingress.submit(&blocked.0);
    assert_eq!(outcome.decision, IngressDecision::Accepted);
    assert_eq!(outcome.accepted_sequence, DEFAULT_MAX_CREDITS as u64 + 1);
}

#[test]
fn a_rejected_packet_costs_no_credit() {
    let mut ingress = ingress();
    let before = ingress.remaining_credits();

    let (outcome, _) = ingress.submit(&[0u8; 4]);
    assert_eq!(outcome.decision, IngressDecision::Rejected);
    assert_eq!(ingress.remaining_credits(), before);

    // Otherwise a producer sending garbage would exhaust the window and stall
    // itself, which reads on device as a hang rather than as bad input.
    for _ in 0..64 {
        let (outcome, _) = ingress.submit(&[0u8; 4]);
        assert_eq!(outcome.decision, IngressDecision::Rejected);
    }
    assert_eq!(ingress.remaining_credits(), before);
}

#[test]
fn a_packet_addressed_to_another_session_is_rejected() {
    let mut ingress = ingress();
    let foreign = build(|mut builder| {
        builder.session_nonce = NONCE ^ 1;
        builder
    });
    let (outcome, frame) = ingress.submit(&foreign);
    assert_eq!(outcome.decision, IngressDecision::Rejected);
    assert_eq!(outcome.wire_error_code, INGRESS_ERROR_FOREIGN_SESSION);
    assert!(frame.is_none());
}

#[test]
fn a_packet_from_a_dead_generation_reports_generation_lost_not_an_error() {
    let mut ingress = ingress();
    let stale = build(|mut builder| {
        builder.runtime_generation = GENERATION - 1;
        builder
    });
    let (outcome, frame) = ingress.submit(&stale);
    // Not Rejected. The producer did nothing wrong, and no retry helps: the
    // WebContent process it was talking to is gone. Reporting this as a
    // sequencing or format error sends whoever reads the telemetry looking for
    // a bug that is not there.
    assert_eq!(outcome.decision, IngressDecision::GenerationLost);
    assert_eq!(outcome.wire_error_code, 0);
    assert!(frame.is_none());
}

#[test]
fn sequences_must_strictly_increase() {
    let mut ingress = ingress();

    let first = packet(10);
    assert_eq!(
        ingress.submit(&first.0).0.decision,
        IngressDecision::Accepted
    );

    for sequence in [10u64, 9, 0] {
        let bytes = packet(sequence);
        let (outcome, frame) = ingress.submit(&bytes.0);
        assert_eq!(
            outcome.decision,
            IngressDecision::Rejected,
            "sequence {sequence} must not be accepted after 10",
        );
        assert_eq!(outcome.wire_error_code, INGRESS_ERROR_STALE_SEQUENCE);
        assert!(frame.is_none());
    }

    let next = packet(11);
    assert_eq!(
        ingress.submit(&next.0).0.decision,
        IngressDecision::Accepted
    );
}

#[test]
fn a_packet_built_against_a_retired_surface_is_rejected() {
    let mut ingress = ingress();
    ingress.set_surface_generation(4);

    let stale = build(|mut builder| {
        builder.surface_generation = 3;
        builder
    });
    let (outcome, _) = ingress.submit(&stale);
    assert_eq!(outcome.decision, IngressDecision::Rejected);
    assert_eq!(outcome.wire_error_code, INGRESS_ERROR_STALE_SURFACE);

    let current = build(|mut builder| {
        builder.surface_generation = 4;
        builder
    });
    assert_eq!(
        ingress.submit(&current).0.decision,
        IngressDecision::Accepted
    );
}

#[test]
fn a_packet_naming_resources_from_before_a_context_loss_is_rejected() {
    let mut ingress = ingress();

    // Resource ids are reused after the table is rebuilt, so a stale id is not
    // an invalid id -- it silently names a different object. The epoch is what
    // makes that detectable at all.
    ingress.set_resource_epoch(2);
    let stale = build(|mut builder| {
        builder.resource_epoch = 1;
        builder
    });
    let (outcome, _) = ingress.submit(&stale);
    assert_eq!(outcome.decision, IngressDecision::Rejected);
    assert_eq!(outcome.wire_error_code, INGRESS_ERROR_STALE_RESOURCE_EPOCH);

    let current = build(|mut builder| {
        builder.resource_epoch = 2;
        builder
    });
    assert_eq!(
        ingress.submit(&current).0.decision,
        IngressDecision::Accepted
    );
}

#[test]
fn validity_does_not_depend_on_how_busy_the_renderer_is() {
    // Same malformed bytes, once with credit available and once without. The
    // answer has to be the same, or a producer would learn that retrying
    // garbage eventually "works" and the failure would surface as a stall.
    let mut ingress = FrameIngress::new(NONCE, GENERATION).with_max_credits(1);
    let malformed = [0u8; 80];

    let (idle, _) = ingress.submit(&malformed);
    assert_eq!(idle.decision, IngressDecision::Rejected);

    let accepted = packet(1);
    assert_eq!(
        ingress.submit(&accepted.0).0.decision,
        IngressDecision::Accepted
    );
    assert_eq!(ingress.remaining_credits(), 0);

    let (busy, _) = ingress.submit(&malformed);
    assert_eq!(busy.decision, IngressDecision::Rejected);
    assert_eq!(busy.wire_error_code, idle.wire_error_code);
}

#[test]
fn double_completion_cannot_turn_backpressure_off() {
    let mut ingress = ingress();
    for _ in 0..16 {
        ingress.complete();
    }
    assert_eq!(ingress.remaining_credits(), DEFAULT_MAX_CREDITS);
    assert_eq!(ingress.in_flight(), 0);

    // Still bounded afterwards: the saturating subtraction must not have left
    // the counter somewhere that lets an unbounded number of frames in.
    for sequence in 1..=DEFAULT_MAX_CREDITS as u64 {
        let bytes = packet(sequence);
        assert_eq!(
            ingress.submit(&bytes.0).0.decision,
            IngressDecision::Accepted
        );
    }
    let overflow = packet(DEFAULT_MAX_CREDITS as u64 + 1);
    assert_eq!(
        ingress.submit(&overflow.0).0.decision,
        IngressDecision::WouldBlock
    );
}
