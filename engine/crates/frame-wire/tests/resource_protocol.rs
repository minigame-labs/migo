//! The resource lane, and the rule that makes it safe: nothing is nameable
//! until it is verified.
//!
//! The failure this protocol exists to prevent is not an upload that goes
//! wrong -- those are cheap and recoverable. It is an upload that goes wrong
//! *and gets used*: a texture whose contents are whatever arrived, already
//! bound by a frame that referenced it, indistinguishable from a correct one
//! except by looking at the screen.

use frame_wire::resource::{
    MAX_CHUNK_BYTES, MAX_RESERVATIONS, MAX_RESOURCE_BYTES, ResourceError, ResourceState,
    ResourceTable,
};

const NOW: u64 = 1_000_000_000;
const DEADLINE: u64 = NOW + 10_000_000_000;
const FORMAT_ASTC: u32 = 4;

fn digest(seed: u8) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = seed.wrapping_add(index as u8);
    }
    out
}

fn table() -> ResourceTable {
    ResourceTable::new(1)
}

#[test]
fn a_resource_becomes_referenceable_only_after_its_digest_is_checked() {
    let mut table = table();
    let expected = digest(0x11);
    let id = table
        .reserve(3000, 3, FORMAT_ASTC, expected, DEADLINE)
        .expect("a well-formed reservation");

    assert_eq!(table.get(id).unwrap().state, ResourceState::Reserved);
    assert!(!table.is_referenceable(id), "nothing has arrived yet");

    assert_eq!(
        table.accept_chunk(id, 0, 1000).unwrap(),
        ResourceState::Uploading
    );
    assert!(!table.is_referenceable(id));
    assert_eq!(
        table.accept_chunk(id, 1, 1000).unwrap(),
        ResourceState::Uploading
    );
    assert_eq!(
        table.accept_chunk(id, 2, 1000).unwrap(),
        ResourceState::Verifying,
        "every chunk arrived, and the digest has not been checked yet"
    );
    assert!(
        !table.is_referenceable(id),
        "a frame must not be able to name bytes that have not been verified"
    );

    table.verify(id, expected).expect("the digest agrees");
    assert_eq!(table.get(id).unwrap().state, ResourceState::Ready);
    assert!(table.is_referenceable(id));
    assert_eq!(table.get(id).unwrap().received_bytes(), 3000);
}

#[test]
fn a_digest_that_does_not_match_fails_the_resource_rather_than_creating_it() {
    let mut table = table();
    let id = table
        .reserve(64, 1, FORMAT_ASTC, digest(0x11), DEADLINE)
        .expect("reserved");
    table.accept_chunk(id, 0, 64).expect("uploaded");

    assert_eq!(
        table.verify(id, digest(0x22)),
        Err(ResourceError::DigestMismatch)
    );
    assert_eq!(table.get(id).unwrap().state, ResourceState::Failed);
    assert_eq!(
        table.get(id).unwrap().error,
        Some(ResourceError::DigestMismatch)
    );
    assert!(!table.is_referenceable(id));

    // And a second attempt with the right digest does not rescue it: the bytes
    // that were staged are the wrong ones.
    assert_eq!(
        table.verify(id, digest(0x11)),
        Err(ResourceError::NotUploading)
    );
    assert!(!table.is_referenceable(id));
}

#[test]
fn verifying_before_every_chunk_arrived_is_refused() {
    let mut table = table();
    let expected = digest(3);
    let id = table
        .reserve(2048, 2, 0, expected, DEADLINE)
        .expect("reserved");
    table.accept_chunk(id, 0, 1024).expect("first chunk");

    assert_eq!(table.verify(id, expected), Err(ResourceError::Incomplete));
    assert_eq!(table.get(id).unwrap().state, ResourceState::Failed);
    assert!(!table.is_referenceable(id));
}

#[test]
fn chunks_must_be_contiguous() {
    let mut table = table();
    let id = table
        .reserve(3072, 3, 0, digest(4), DEADLINE)
        .expect("reserved");
    table.accept_chunk(id, 0, 1024).expect("first");

    assert_eq!(
        table.accept_chunk(id, 2, 1024),
        Err(ResourceError::NonContiguousChunk),
        "a gap means a chunk was lost, and there is no recovery but starting over"
    );
    assert_eq!(table.get(id).unwrap().state, ResourceState::Failed);
    // Replaying an earlier chunk is the same rejection.
    let id = table
        .reserve(3072, 3, 0, digest(5), DEADLINE)
        .expect("reserved");
    table.accept_chunk(id, 0, 1024).expect("first");
    table.accept_chunk(id, 1, 1024).expect("second");
    assert_eq!(
        table.accept_chunk(id, 1, 1024),
        Err(ResourceError::NonContiguousChunk)
    );
}

#[test]
fn a_chunk_may_not_be_oversized_or_run_past_the_declared_size() {
    let mut table = table();
    let id = table
        .reserve(2048, 2, 0, digest(6), DEADLINE)
        .expect("reserved");
    assert_eq!(
        table.accept_chunk(id, 0, MAX_CHUNK_BYTES + 1),
        Err(ResourceError::ChunkOutOfBounds)
    );

    let id = table
        .reserve(2048, 2, 0, digest(7), DEADLINE)
        .expect("reserved");
    table.accept_chunk(id, 0, 1024).expect("first");
    assert_eq!(
        table.accept_chunk(id, 1, 2048),
        Err(ResourceError::ChunkOutOfBounds),
        "the upload may not exceed the size it declared"
    );

    let id = table
        .reserve(2048, 2, 0, digest(8), DEADLINE)
        .expect("reserved");
    assert_eq!(
        table.accept_chunk(id, 0, 0),
        Err(ResourceError::ChunkOutOfBounds),
        "an empty chunk advances the index without advancing the upload"
    );
}

#[test]
fn a_reservation_the_producer_cannot_fulfil_is_refused_at_the_start() {
    let mut table = table();
    for (bytes, chunks, expected) in [
        (0u64, 1u32, ResourceError::BadSize),
        (MAX_RESOURCE_BYTES + 1, 128, ResourceError::BadSize),
        (2048, 0, ResourceError::BadChunkCount),
        // One chunk cannot carry more than MAX_CHUNK_BYTES, so this declaration
        // could never complete and would hold its slot until the deadline.
        (
            u64::from(MAX_CHUNK_BYTES) + 1,
            1,
            ResourceError::BadChunkCount,
        ),
    ] {
        assert_eq!(
            table.reserve(bytes, chunks, 0, digest(9), DEADLINE),
            Err(expected),
            "reserve({bytes}, {chunks})"
        );
    }
    assert_eq!(table.open_reservations(), 0);
}

#[test]
fn the_table_is_bounded() {
    let mut table = table();
    for index in 0..MAX_RESERVATIONS {
        table
            .reserve(64, 1, 0, digest(index as u8), DEADLINE)
            .unwrap_or_else(|error| panic!("reservation {index} refused: {error}"));
    }
    assert_eq!(
        table.reserve(64, 1, 0, digest(0), DEADLINE),
        Err(ResourceError::TooManyReservations)
    );
    assert_eq!(table.open_reservations(), MAX_RESERVATIONS);
}

#[test]
fn an_upload_that_misses_its_deadline_fails() {
    let mut table = table();
    let id = table
        .reserve(2048, 2, 0, digest(10), DEADLINE)
        .expect("reserved");
    table.accept_chunk(id, 0, 1024).expect("first");

    assert_eq!(table.expire(DEADLINE - 1), 0);
    assert_eq!(table.get(id).unwrap().state, ResourceState::Uploading);

    assert_eq!(table.expire(DEADLINE), 1);
    assert_eq!(table.get(id).unwrap().state, ResourceState::Failed);
    assert_eq!(table.get(id).unwrap().error, Some(ResourceError::TimedOut));
    assert_eq!(table.expire(DEADLINE + 1), 0, "already settled");
}

#[test]
fn a_context_loss_discards_every_resource() {
    let mut table = table();
    let expected = digest(11);
    let id = table
        .reserve(64, 1, 0, expected, DEADLINE)
        .expect("reserved");
    table.accept_chunk(id, 0, 64).expect("uploaded");
    table.verify(id, expected).expect("verified");
    assert!(table.is_referenceable(id));

    let discarded = table.advance_epoch(2).expect("the epoch only advances");
    assert_eq!(discarded, 1);
    assert_eq!(table.epoch(), 2);
    assert!(
        !table.is_referenceable(id),
        "the ids in a rebuilt table name different objects"
    );
    assert_eq!(table.open_reservations(), 0);
    assert_eq!(
        table.accept_chunk(id, 0, 64),
        Err(ResourceError::UnknownReservation)
    );
}

#[test]
fn the_epoch_only_advances() {
    let mut table = ResourceTable::new(5);
    assert_eq!(table.advance_epoch(4), Err(ResourceError::EpochAdvanced));
    assert_eq!(table.epoch(), 5, "and nothing was discarded");
    assert_eq!(
        table.advance_epoch(5).expect("standing still is allowed"),
        0
    );
    assert!(table.advance_epoch(6).is_ok());
}

#[test]
fn releasing_a_resource_makes_it_unnameable() {
    let mut table = table();
    let expected = digest(12);
    let id = table
        .reserve(64, 1, 0, expected, DEADLINE)
        .expect("reserved");
    table.accept_chunk(id, 0, 64).expect("uploaded");
    table.verify(id, expected).expect("verified");

    assert!(table.release(id));
    assert!(!table.is_referenceable(id));
    assert!(
        !table.release(id),
        "releasing twice is not an error and not a second release"
    );
}

#[test]
fn an_unknown_reservation_is_never_referenceable() {
    let table = table();
    assert!(!table.is_referenceable(1));
    assert!(!table.is_referenceable(u64::MAX));
    assert!(table.get(1).is_none());
}
