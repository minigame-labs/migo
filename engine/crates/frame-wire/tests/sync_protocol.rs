//! The synchronous barrier, and every way a blocked producer gets woken.
//!
//! The cases that matter here are the failures. A producer inside
//! `Atomics.wait` has stopped; whether it starts again is decided entirely by
//! whether one of these paths settles its request. A path that forgets leaves
//! an agent blocked until its process is reclaimed, which on a phone is a game
//! that stopped drawing and never said why.

use frame_wire::sync::{
    MAX_IN_FLIGHT, MAX_REPLY_BYTES, SYNC_LAYOUT, SYNC_RECORD_BYTES, SyncError, SyncMailbox,
    SyncRequest, SyncState,
};

const GENERATION: u64 = 7;
const NOW: u64 = 1_000_000_000;
const OP_READ_PIXELS: u32 = 1;

fn request() -> SyncRequest {
    SyncRequest {
        request_id: 0,
        runtime_generation: GENERATION,
        surface_generation: 3,
        resource_epoch: 2,
        triggering_sequence: 41,
        operation: OP_READ_PIXELS,
        max_reply_bytes: 4096,
        deadline_nanos: NOW + 100_000_000,
    }
}

fn mailbox() -> SyncMailbox {
    SyncMailbox::new(GENERATION)
}

#[test]
fn the_record_layout_is_gapless_and_matches_the_declared_size() {
    let mut expected = 0u32;
    for field in SYNC_LAYOUT {
        assert_eq!(
            field.offset, expected,
            "{} starts at {} but the previous field ends at {expected}",
            field.name, field.offset
        );
        expected += field.size;
    }
    assert_eq!(expected, SYNC_RECORD_BYTES);
    // The state is first and four bytes wide because the producer reads it with
    // an atomic load out of a shared cell.
    assert_eq!(SYNC_LAYOUT[0].name, "state");
    assert_eq!(SYNC_LAYOUT[0].offset, 0);
    assert_eq!(SYNC_LAYOUT[0].size, 4);
}

#[test]
fn a_request_is_accepted_answered_and_acknowledged() {
    let mut mailbox = mailbox();
    assert_eq!(mailbox.state(), SyncState::Free);

    let id = mailbox
        .post(request(), NOW)
        .expect("a first request is accepted");
    assert_ne!(
        id, 0,
        "zero is what a cleared mailbox holds, never a request"
    );
    assert_eq!(mailbox.state(), SyncState::Pending);
    assert!(!mailbox.state().is_settled());

    mailbox
        .complete(id, 4096)
        .expect("a reply that fits is accepted");
    assert_eq!(mailbox.state(), SyncState::Ready);
    assert_eq!(mailbox.reply_bytes(), 4096);
    assert!(mailbox.state().is_settled());

    mailbox.acknowledge();
    assert_eq!(mailbox.state(), SyncState::Free);
    assert_eq!(mailbox.reply_bytes(), 0);
    assert!(mailbox.request().is_none());
}

#[test]
fn only_one_request_may_be_outstanding() {
    assert_eq!(MAX_IN_FLIGHT, 1);
    let mut mailbox = mailbox();
    mailbox.post(request(), NOW).expect("first");
    assert_eq!(
        mailbox.post(request(), NOW),
        Err(SyncError::AlreadyPending),
        "a second request would need a second mailbox and a second waiter"
    );
    // And the first is untouched: a refused post must not disturb what is
    // already blocking a producer.
    assert_eq!(mailbox.state(), SyncState::Pending);
}

#[test]
fn a_reply_that_answers_another_request_is_refused() {
    let mut mailbox = mailbox();
    let id = mailbox.post(request(), NOW).expect("posted");
    assert_eq!(
        mailbox.complete(id.wrapping_add(1), 16),
        Err(SyncError::RequestIdMismatch)
    );
    // The request fails rather than staying pending: a host that answered the
    // wrong id is a host whose next answer cannot be trusted either.
    assert_eq!(mailbox.state(), SyncState::Failed);
    assert_eq!(mailbox.error(), Some(SyncError::RequestIdMismatch));
    assert_eq!(mailbox.reply_bytes(), 0);
}

#[test]
fn a_reply_larger_than_the_producer_reserved_is_refused_rather_than_truncated() {
    let mut mailbox = mailbox();
    let mut request = request();
    request.max_reply_bytes = 1024;
    let id = mailbox.post(request, NOW).expect("posted");

    assert_eq!(mailbox.complete(id, 1025), Err(SyncError::ReplyTooLarge));
    assert_eq!(mailbox.state(), SyncState::Failed);
    assert_eq!(
        mailbox.reply_bytes(),
        0,
        "a truncated readPixels is a wrong answer that looks like a right one"
    );
}

#[test]
fn a_reservation_outside_the_protocol_is_refused_before_anything_blocks() {
    let mut mailbox = mailbox();
    for bad in [0, MAX_REPLY_BYTES + 1] {
        let mut request = request();
        request.max_reply_bytes = bad;
        assert_eq!(
            mailbox.post(request, NOW),
            Err(SyncError::BadReplyReservation)
        );
        assert_eq!(mailbox.state(), SyncState::Free, "nothing was left pending");
    }
}

#[test]
fn a_deadline_that_is_not_in_the_future_is_refused() {
    let mut mailbox = mailbox();
    for deadline in [0, NOW - 1, NOW] {
        let mut request = request();
        request.deadline_nanos = deadline;
        assert_eq!(mailbox.post(request, NOW), Err(SyncError::BadDeadline));
        assert_eq!(mailbox.state(), SyncState::Free);
    }
}

#[test]
fn a_passed_deadline_wakes_the_waiter() {
    let mut mailbox = mailbox();
    let request = request();
    mailbox.post(request, NOW).expect("posted");

    assert!(!mailbox.expire_if_due(request.deadline_nanos - 1));
    assert_eq!(mailbox.state(), SyncState::Pending);

    assert!(mailbox.expire_if_due(request.deadline_nanos));
    assert_eq!(mailbox.state(), SyncState::Failed);
    assert_eq!(mailbox.error(), Some(SyncError::TimedOut));

    // Expiring twice settles nothing further.
    assert!(!mailbox.expire_if_due(request.deadline_nanos + 1));
}

#[test]
fn a_reply_after_the_request_settled_is_refused() {
    let mut mailbox = mailbox();
    let request = request();
    let id = mailbox.post(request, NOW).expect("posted");
    assert!(mailbox.expire_if_due(request.deadline_nanos));

    assert_eq!(
        mailbox.complete(id, 16),
        Err(SyncError::LateReply),
        "the producer has moved on and its reply buffer may be someone else's"
    );
    assert_eq!(
        mailbox.error(),
        Some(SyncError::TimedOut),
        "the reason is unchanged"
    );
}

#[test]
fn a_generation_move_under_a_waiter_fails_it() {
    let mut mailbox = mailbox();
    mailbox.post(request(), NOW).expect("posted");
    assert!(mailbox.invalidate());
    assert_eq!(mailbox.state(), SyncState::Failed);
    assert_eq!(mailbox.error(), Some(SyncError::StaleGeneration));
    assert!(!mailbox.invalidate(), "there is nothing left to invalidate");
}

#[test]
fn a_request_built_against_a_dead_generation_is_refused() {
    let mut mailbox = mailbox();
    let mut request = request();
    request.runtime_generation = GENERATION + 1;
    assert_eq!(mailbox.post(request, NOW), Err(SyncError::StaleGeneration));
    assert_eq!(mailbox.state(), SyncState::Free);
}

#[test]
fn teardown_wakes_the_waiter_and_refuses_every_later_request() {
    let mut mailbox = mailbox();
    mailbox.post(request(), NOW).expect("posted");

    assert!(mailbox.end_session());
    assert_eq!(mailbox.state(), SyncState::Failed);
    assert_eq!(
        mailbox.error(),
        Some(SyncError::SessionEnded),
        "a producer inside Atomics.wait on a session that is gone stays blocked \
         until its agent is destroyed"
    );

    mailbox.acknowledge();
    assert_eq!(
        mailbox.post(request(), NOW),
        Err(SyncError::SessionEnded),
        "a request posted after teardown would block on nothing"
    );
}

#[test]
fn a_cancelled_request_settles_without_an_error() {
    let mut mailbox = mailbox();
    let id = mailbox.post(request(), NOW).expect("posted");
    assert!(mailbox.cancel());
    assert_eq!(mailbox.state(), SyncState::Cancelled);
    assert_eq!(
        mailbox.error(),
        None,
        "the producer withdrew; nothing went wrong"
    );
    assert_eq!(mailbox.complete(id, 8), Err(SyncError::LateReply));
    assert!(!mailbox.cancel());
}

#[test]
fn request_ids_advance_and_never_reuse_zero() {
    let mut mailbox = mailbox();
    let mut seen = Vec::new();
    for _ in 0..4 {
        let id = mailbox.post(request(), NOW).expect("posted");
        seen.push(id);
        mailbox.complete(id, 8).expect("answered");
        mailbox.acknowledge();
    }
    assert!(seen.iter().all(|id| *id != 0));
    let mut sorted = seen.clone();
    sorted.dedup();
    assert_eq!(sorted.len(), seen.len(), "ids are not reused back to back");
}

/// Every settled state must be reachable, or one of them is a branch nothing
/// exercises and the producer's matching arm is dead too.
#[test]
fn every_settled_state_is_reachable() {
    let mut ready = mailbox();
    let id = ready.post(request(), NOW).expect("posted");
    ready.complete(id, 8).expect("answered");
    assert_eq!(ready.state(), SyncState::Ready);

    let mut failed = mailbox();
    failed.post(request(), NOW).expect("posted");
    failed.invalidate();
    assert_eq!(failed.state(), SyncState::Failed);

    let mut cancelled = mailbox();
    cancelled.post(request(), NOW).expect("posted");
    cancelled.cancel();
    assert_eq!(cancelled.state(), SyncState::Cancelled);

    for state in [SyncState::Ready, SyncState::Failed, SyncState::Cancelled] {
        assert!(state.is_settled());
        assert_eq!(SyncState::from_code(state.code()), Some(state));
    }
    assert!(!SyncState::Free.is_settled());
    assert!(!SyncState::Pending.is_settled());
    assert_eq!(SyncState::from_code(99), None);
}
