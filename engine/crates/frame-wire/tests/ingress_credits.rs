//! Identity, ordering, admission and backpressure: the rules the parser cannot
//! check because they depend on state the host owns.

use frame_wire::{
    FrameIngress, IngressDecision, MAX_TOTAL_BYTES, SECTION_KIND_COMMAND_STREAM,
    SECTION_KIND_RESOURCE_REFERENCES, WireError,
    builder::WireFrameBuilder,
    ingress::{
        DEFAULT_MAX_CREDITS, INGRESS_ERROR_FOREIGN_SESSION, INGRESS_ERROR_NONCONTIGUOUS_SEQUENCE,
        INGRESS_ERROR_PACKET_TOO_LARGE, INGRESS_ERROR_RESOURCES_NOT_READY,
        INGRESS_ERROR_STALE_RESOURCE_EPOCH, INGRESS_ERROR_STALE_SURFACE, MAX_CREDITS,
    },
};

const NONCE: u128 = 0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210;
const GENERATION: u64 = 7;

type Packet = Vec<u8>;

fn packet(sequence: u64) -> Packet {
    build(sequence, NONCE, GENERATION, 0, 0)
}

fn build(
    sequence: u64,
    launch_nonce: u128,
    runtime_generation: u64,
    surface_generation: u64,
    resource_epoch: u64,
) -> Packet {
    let stream = [0u8; 8];
    let mut frame = WireFrameBuilder::new();
    frame.sequence = sequence;
    frame.launch_nonce = launch_nonce;
    frame.runtime_generation = runtime_generation;
    frame.surface_generation = surface_generation;
    frame.resource_epoch = resource_epoch;
    frame.frame_id = sequence as u32;
    frame
        .section(SECTION_KIND_COMMAND_STREAM, 2, &stream)
        .build()
}

/// A packet that names resources, for the admission cases.
fn packet_with_resources(sequence: u64, resource_epoch: u64) -> Packet {
    let stream = [0u8; 8];
    let refs = [0u8; 8];
    let mut frame = WireFrameBuilder::new();
    frame.sequence = sequence;
    frame.launch_nonce = NONCE;
    frame.runtime_generation = GENERATION;
    frame.resource_epoch = resource_epoch;
    frame
        .section(SECTION_KIND_COMMAND_STREAM, 2, &stream)
        .section(SECTION_KIND_RESOURCE_REFERENCES, 2, &refs)
        .build()
}

fn ingress() -> FrameIngress {
    FrameIngress::new(NONCE, GENERATION)
}

#[test]
fn credits_bound_the_number_of_frames_in_flight() {
    let mut ingress = ingress();
    assert_eq!(ingress.remaining_credits(), DEFAULT_MAX_CREDITS);

    let mut sequence = 1;
    for expected_remaining in (0..DEFAULT_MAX_CREDITS).rev() {
        let bytes = packet(sequence);
        let (outcome, frame) = ingress.submit(&bytes);
        assert_eq!(outcome.decision, IngressDecision::Accepted);
        assert_eq!(outcome.remaining_credits, expected_remaining);
        assert_eq!(outcome.accepted_sequence, sequence);
        assert!(frame.is_some());
        sequence += 1;
    }

    let bytes = packet(sequence);
    let (outcome, frame) = ingress.submit(&bytes);
    assert_eq!(outcome.decision, IngressDecision::WouldBlock);
    assert_eq!(outcome.remaining_credits, 0);
    assert!(frame.is_none());

    // The blocked packet keeps its number: WouldBlock consumes nothing, so the
    // producer resends the same bytes rather than skipping a sequence it can
    // never fill.
    ingress.complete();
    let (outcome, frame) = ingress.submit(&bytes);
    assert_eq!(outcome.decision, IngressDecision::Accepted);
    assert_eq!(outcome.accepted_sequence, sequence);
    assert!(frame.is_some());
}

#[test]
fn a_rejected_packet_costs_no_credit() {
    let mut ingress = ingress();
    let mut bytes = packet(1);
    bytes[0] ^= 0xFF;

    let (outcome, frame) = ingress.submit(&bytes);
    assert_eq!(outcome.decision, IngressDecision::Rejected);
    assert_eq!(outcome.wire_error_code, WireError::BadMagic.code());
    assert_eq!(outcome.remaining_credits, DEFAULT_MAX_CREDITS);
    assert!(frame.is_none());
    assert_eq!(ingress.in_flight(), 0);
}

#[test]
fn a_packet_addressed_to_another_launch_is_rejected() {
    let mut ingress = ingress();
    // Only the high half differs: a 64-bit comparison would accept this.
    let neighbour = NONCE ^ (1u128 << 100);
    let bytes = build(1, neighbour, GENERATION, 0, 0);
    let (outcome, frame) = ingress.submit(&bytes);
    assert_eq!(outcome.decision, IngressDecision::Rejected);
    assert_eq!(outcome.wire_error_code, INGRESS_ERROR_FOREIGN_SESSION);
    assert!(frame.is_none());
}

#[test]
fn a_packet_from_a_dead_generation_reports_generation_lost_not_an_error() {
    let mut ingress = ingress();
    // Only the high half differs here too.
    let bytes = build(1, NONCE, GENERATION | (1u64 << 40), 0, 0);
    let (outcome, frame) = ingress.submit(&bytes);
    assert_eq!(outcome.decision, IngressDecision::GenerationLost);
    assert_eq!(
        outcome.wire_error_code, 0,
        "generation loss is nobody's error"
    );
    assert!(frame.is_none());
}

/// Strictly contiguous, not merely increasing. A gap means a packet carrying
/// state was lost, and there is no recovery from that which does not involve a
/// new generation.
#[test]
fn sequences_must_be_strictly_contiguous() {
    let mut ingress = ingress();

    let first = packet(1);
    assert_eq!(
        ingress.submit(&first).0.decision,
        IngressDecision::Accepted,
        "the first accepted sequence is 1"
    );
    ingress.complete();

    for wrong in [1u64, 0, 3, 100, u64::MAX] {
        let bytes = packet(wrong);
        let (outcome, frame) = ingress.submit(&bytes);
        assert_eq!(
            outcome.decision,
            IngressDecision::Rejected,
            "sequence {wrong} is not exactly 2"
        );
        assert_eq!(
            outcome.wire_error_code,
            INGRESS_ERROR_NONCONTIGUOUS_SEQUENCE
        );
        assert!(frame.is_none());
        assert_eq!(ingress.last_accepted_sequence(), 1);
    }

    let next = packet(2);
    assert_eq!(ingress.submit(&next).0.decision, IngressDecision::Accepted);
}

/// A producer cannot start anywhere it likes: the first packet of a generation
/// is sequence 1, so a replay of a later frame from a previous generation --
/// same nonce, same generation number reused -- has nothing to land on.
#[test]
fn the_first_packet_of_a_generation_must_be_sequence_one() {
    let mut ingress = ingress();
    let bytes = packet(2);
    let (outcome, _) = ingress.submit(&bytes);
    assert_eq!(outcome.decision, IngressDecision::Rejected);
    assert_eq!(
        outcome.wire_error_code,
        INGRESS_ERROR_NONCONTIGUOUS_SEQUENCE
    );
}

#[test]
fn a_packet_built_against_a_retired_surface_is_rejected() {
    let mut ingress = ingress();
    assert!(ingress.set_surface_generation(4));

    let stale = build(1, NONCE, GENERATION, 3, 0);
    let (outcome, frame) = ingress.submit(&stale);
    assert_eq!(outcome.decision, IngressDecision::Rejected);
    assert_eq!(outcome.wire_error_code, INGRESS_ERROR_STALE_SURFACE);
    assert!(frame.is_none());

    let current = build(1, NONCE, GENERATION, 4, 0);
    assert_eq!(
        ingress.submit(&current).0.decision,
        IngressDecision::Accepted
    );
}

#[test]
fn a_packet_naming_resources_from_before_a_context_loss_is_rejected() {
    let mut ingress = ingress();
    assert!(ingress.set_resource_epoch(2));
    ingress.mark_resources_ready();

    let stale = build(1, NONCE, GENERATION, 0, 1);
    let (outcome, frame) = ingress.submit(&stale);
    assert_eq!(outcome.decision, IngressDecision::Rejected);
    assert_eq!(outcome.wire_error_code, INGRESS_ERROR_STALE_RESOURCE_EPOCH);
    assert!(frame.is_none());

    let current = build(1, NONCE, GENERATION, 0, 2);
    assert_eq!(
        ingress.submit(&current).0.decision,
        IngressDecision::Accepted
    );
}

/// Neither timeline may move backwards, and the refusal is observable. A
/// timeline that can go back is a timeline on which a stale packet becomes
/// valid again.
#[test]
fn the_surface_and_resource_timelines_only_advance() {
    let mut ingress = ingress();
    assert!(ingress.set_surface_generation(5));
    assert!(!ingress.set_surface_generation(4), "backwards is refused");
    assert_eq!(ingress.surface_generation(), 5, "and changes nothing");
    assert!(
        ingress.set_surface_generation(5),
        "standing still is allowed"
    );
    assert!(ingress.set_surface_generation(6));

    assert!(ingress.set_resource_epoch(9));
    assert!(!ingress.set_resource_epoch(8));
    assert_eq!(ingress.resource_epoch(), 9);
    assert!(ingress.set_resource_epoch(10));
}

/// Advancing the epoch clears readiness in the same call. A host that had to
/// remember to clear it separately is a host that will eventually forget, and
/// the failure would be a frame drawing with ids that name whatever the rebuilt
/// table put in their place.
#[test]
fn advancing_the_resource_epoch_withdraws_readiness() {
    let mut ingress = ingress();
    assert!(!ingress.resources_ready(), "nothing is ready at the start");

    let early = packet_with_resources(1, 0);
    let (outcome, frame) = ingress.submit(&early);
    assert_eq!(outcome.decision, IngressDecision::Rejected);
    assert_eq!(outcome.wire_error_code, INGRESS_ERROR_RESOURCES_NOT_READY);
    assert!(frame.is_none());

    ingress.mark_resources_ready();
    let admitted = packet_with_resources(1, 0);
    assert_eq!(
        ingress.submit(&admitted).0.decision,
        IngressDecision::Accepted
    );
    ingress.complete();

    // Context loss: the table is rebuilt, so nothing in it is ready.
    assert!(ingress.set_resource_epoch(1));
    assert!(!ingress.resources_ready());
    let after_loss = packet_with_resources(2, 1);
    let (outcome, _) = ingress.submit(&after_loss);
    assert_eq!(outcome.wire_error_code, INGRESS_ERROR_RESOURCES_NOT_READY);

    // A frame that names nothing is still fine while the table is rebuilding.
    let plain = build(2, NONCE, GENERATION, 0, 1);
    assert_eq!(ingress.submit(&plain).0.decision, IngressDecision::Accepted);
}

/// Setting the same epoch again is not an advance and must not withdraw
/// readiness -- otherwise an idempotent host call would silently stall the
/// producer.
#[test]
fn restating_the_current_epoch_leaves_readiness_alone() {
    let mut ingress = ingress();
    assert!(ingress.set_resource_epoch(3));
    ingress.mark_resources_ready();
    assert!(ingress.set_resource_epoch(3));
    assert!(ingress.resources_ready());
}

#[test]
fn validity_does_not_depend_on_how_busy_the_renderer_is() {
    let mut ingress = ingress();
    let mut sequence = 1;
    for _ in 0..DEFAULT_MAX_CREDITS {
        let bytes = packet(sequence);
        assert_eq!(ingress.submit(&bytes).0.decision, IngressDecision::Accepted);
        sequence += 1;
    }
    assert_eq!(ingress.remaining_credits(), 0);

    // Out of credit, but malformed bytes are still rejected rather than told to
    // wait. Answering WouldBlock here invites the producer to resend garbage
    // forever.
    let mut bad = packet(sequence);
    bad[4] ^= 0xFF;
    let (outcome, _) = ingress.submit(&bad);
    assert_eq!(outcome.decision, IngressDecision::Rejected);
    assert_eq!(
        outcome.wire_error_code,
        WireError::UnsupportedVersion.code()
    );

    // So is a foreign packet, and so is a non-contiguous one.
    let foreign = build(sequence, NONCE ^ 1, GENERATION, 0, 0);
    assert_eq!(
        ingress.submit(&foreign).0.wire_error_code,
        INGRESS_ERROR_FOREIGN_SESSION
    );
    let jumped = packet(sequence + 5);
    assert_eq!(
        ingress.submit(&jumped).0.wire_error_code,
        INGRESS_ERROR_NONCONTIGUOUS_SEQUENCE
    );
}

#[test]
fn double_completion_cannot_turn_backpressure_off() {
    let mut ingress = ingress();
    let bytes = packet(1);
    assert_eq!(ingress.submit(&bytes).0.decision, IngressDecision::Accepted);
    assert_eq!(ingress.in_flight(), 1);

    ingress.complete();
    ingress.complete();
    ingress.complete();
    assert_eq!(ingress.in_flight(), 0);
    assert_eq!(ingress.remaining_credits(), DEFAULT_MAX_CREDITS);
}

/// The credit window has a compile-time ceiling, and the setter clamps to it.
/// A value that arrives from configuration, a remote policy or a content
/// manifest can only tighten the window.
#[test]
fn the_credit_window_cannot_be_widened_by_a_caller() {
    for requested in [MAX_CREDITS + 1, 64, u32::MAX] {
        let ingress = ingress().with_max_credits(requested);
        assert_eq!(
            ingress.max_credits(),
            MAX_CREDITS,
            "requesting {requested} credits must clamp to the ceiling"
        );
    }
    assert_eq!(ingress().with_max_credits(1).max_credits(), 1);
    assert_eq!(
        ingress().with_max_credits(0).max_credits(),
        1,
        "zero would deadlock the producer rather than throttle it"
    );
}

/// The packet ceiling behaves the same way: lowerable, never raisable.
#[test]
fn the_packet_ceiling_cannot_be_raised_by_a_caller() {
    for requested in [MAX_TOTAL_BYTES + 1, u32::MAX] {
        let ingress = ingress().with_max_packet_bytes(requested);
        assert_eq!(ingress.max_packet_bytes(), MAX_TOTAL_BYTES);
    }

    let ingress = ingress().with_max_packet_bytes(4096);
    assert_eq!(ingress.max_packet_bytes(), 4096);
}

#[test]
fn a_packet_above_this_sessions_ceiling_is_refused_before_it_is_parsed() {
    let stream = vec![0u8; 4096];
    let mut frame = WireFrameBuilder::new();
    frame.launch_nonce = NONCE;
    frame.runtime_generation = GENERATION;
    let big = frame
        .section(SECTION_KIND_COMMAND_STREAM, 1024, &stream)
        .build();

    let mut tight = ingress().with_max_packet_bytes(1024);
    let (outcome, returned) = tight.submit(&big);
    assert_eq!(outcome.decision, IngressDecision::Rejected);
    assert_eq!(outcome.wire_error_code, INGRESS_ERROR_PACKET_TOO_LARGE);
    assert!(returned.is_none());
    assert_eq!(outcome.remaining_credits, DEFAULT_MAX_CREDITS);

    // The same bytes are fine for a session that did not tighten.
    let mut default = ingress();
    assert_eq!(default.submit(&big).0.decision, IngressDecision::Accepted);
}
