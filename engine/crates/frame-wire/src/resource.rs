//! The resource lane: large bytes, uploaded out of band, referenceable only
//! after they are verified.
//!
//! A frame packet is small and bounded, and a texture atlas is neither. So
//! large assets travel on their own path -- reserved, uploaded in chunks,
//! verified against a digest the producer declared up front, and only then
//! nameable from a frame. The ceiling on a frame packet stays small precisely
//! because this exists.
//!
//! # Why a resource is not ready until it is verified
//!
//! The alternative is to create the GPU object as the bytes arrive and fix it
//! up if the digest turns out wrong. That trades a bounded failure -- the
//! upload failed, nothing was created -- for an unbounded one: a texture whose
//! contents are whatever arrived, already bound by a frame that referenced it,
//! with no way to tell the difference from a correct one except by looking at
//! the screen. Verification before creation is what makes "the producer sent
//! the wrong bytes" a rejection instead of a rendering artefact.
//!
//! # Why the digest is computed by the caller
//!
//! This crate has one dependency, a CRC implementation, because it parses bytes
//! produced by content JavaScript in another process and every dependency is
//! another thing inside that boundary. A SHA-256 implementation here would be a
//! second. The bytes are already in the host's hands when they arrive, and the
//! host already links a hash for its install-integrity work, so the digest is
//! computed there and compared here. This module owns the protocol; it does not
//! own the arithmetic.

use core::fmt;

/// The largest single resource. Big enough for a full compressed atlas, small
/// enough that a producer cannot name a reservation that will not fit in the
/// memory this lane exists to save.
pub const MAX_RESOURCE_BYTES: u64 = 64 * 1024 * 1024;

/// The largest single chunk. One chunk is copied at a time, so this is also the
/// staging buffer's size.
pub const MAX_CHUNK_BYTES: u32 = 1024 * 1024;

/// The most reservations a session may have open at once.
pub const MAX_RESERVATIONS: usize = 64;

/// Where a resource is.
///
/// The numbers cross to the producer, which reports upload progress from them.
/// Never renumber; only append.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ResourceState {
    /// Declared: size, digest and format are known, no bytes have arrived.
    Reserved = 0,
    /// Some chunks have arrived, and not all of them.
    Uploading = 1,
    /// Every chunk arrived; the digest has not been checked yet.
    Verifying = 2,
    /// Verified. A frame may name this resource, and not before.
    Ready = 3,
    /// It will not become ready. `error` says why.
    Failed = 4,
}

impl ResourceState {
    /// Every state, for consumers that must cover all of them.
    ///
    /// Checked against this file's source by
    /// `tests/wire_document_agreement.rs`: a variant added without being
    /// listed here breaks that test rather than quietly escaping every
    /// consumer that iterates this.
    pub const ALL: &'static [ResourceState] = &[
        Self::Reserved,
        Self::Uploading,
        Self::Verifying,
        Self::Ready,
        Self::Failed,
    ];

    #[inline]
    pub const fn code(self) -> u32 {
        self as u32
    }
}

/// Why a resource upload failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ResourceError {
    /// The session already has as many reservations open as it may.
    TooManyReservations = 1,
    /// The declared size is zero or above the ceiling.
    BadSize = 2,
    /// The declared chunk count cannot cover the declared size, or is zero.
    BadChunkCount = 3,
    /// A chunk arrived for a reservation that does not exist.
    UnknownReservation = 4,
    /// A chunk arrived out of order. Chunks are strictly contiguous.
    NonContiguousChunk = 5,
    /// A chunk is larger than the protocol allows, or would carry the upload
    /// past the size the reservation declared.
    ChunkOutOfBounds = 6,
    /// The bytes hash to something other than what was declared.
    DigestMismatch = 7,
    /// The upload did not finish before its deadline.
    TimedOut = 8,
    /// The resource epoch moved: the table it belonged to was rebuilt.
    EpochAdvanced = 9,
    /// The upload was finished with fewer chunks than it declared.
    Incomplete = 10,
    /// A reservation was made in a state that does not allow one.
    NotUploading = 11,
}

impl ResourceError {
    /// Every failure, for the C ABI mirror and its coverage test.
    ///
    /// Checked against this file's source by
    /// `tests/wire_document_agreement.rs`: a variant added without being
    /// listed here breaks that test rather than quietly escaping every
    /// consumer that iterates this.
    pub const ALL: &'static [ResourceError] = &[
        Self::TooManyReservations,
        Self::BadSize,
        Self::BadChunkCount,
        Self::UnknownReservation,
        Self::NonContiguousChunk,
        Self::ChunkOutOfBounds,
        Self::DigestMismatch,
        Self::TimedOut,
        Self::EpochAdvanced,
        Self::Incomplete,
        Self::NotUploading,
    ];

    #[inline]
    pub const fn code(self) -> u32 {
        self as u32
    }
}

impl fmt::Display for ResourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooManyReservations => "the session has too many open reservations",
            Self::BadSize => "the declared resource size is zero or above the ceiling",
            Self::BadChunkCount => "the declared chunk count cannot cover the declared size",
            Self::UnknownReservation => "no such reservation",
            Self::NonContiguousChunk => "chunks must arrive in order with no gaps",
            Self::ChunkOutOfBounds => "the chunk is oversized or runs past the declared size",
            Self::DigestMismatch => "the uploaded bytes do not match the declared digest",
            Self::TimedOut => "the upload did not finish before its deadline",
            Self::EpochAdvanced => "the resource table was rebuilt under this upload",
            Self::Incomplete => "the upload was finished with chunks missing",
            Self::NotUploading => "the reservation is not accepting chunks",
        })
    }
}

/// One declared resource.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reservation {
    pub id: u64,
    pub total_bytes: u64,
    pub chunk_count: u32,
    /// The epoch this resource belongs to. An epoch advance destroys it: the
    /// ids in the rebuilt table mean different things.
    pub resource_epoch: u64,
    /// Producer-declared format tag. Opaque here; the decoder knows what it
    /// means, and the protocol only carries it.
    pub format: u32,
    pub sha256: [u8; 32],
    pub deadline_nanos: u64,
    pub state: ResourceState,
    pub error: Option<ResourceError>,
    received_bytes: u64,
    next_chunk: u32,
}

impl Reservation {
    #[inline]
    pub const fn received_bytes(&self) -> u64 {
        self.received_bytes
    }

    #[inline]
    pub const fn next_chunk(&self) -> u32 {
        self.next_chunk
    }

    /// Whether a frame may name this resource.
    #[inline]
    pub const fn is_referenceable(&self) -> bool {
        matches!(self.state, ResourceState::Ready)
    }
}

/// The session's open reservations.
///
/// Bounded in every dimension a producer can influence: how many reservations,
/// how large each one is, how large a chunk is, and how long an upload may take.
#[derive(Debug, Default)]
pub struct ResourceTable {
    entries: Vec<Reservation>,
    epoch: u64,
    next_id: u64,
}

impl ResourceTable {
    pub fn new(resource_epoch: u64) -> Self {
        Self {
            entries: Vec::new(),
            epoch: resource_epoch,
            next_id: 1,
        }
    }

    #[inline]
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    #[inline]
    pub fn open_reservations(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, id: u64) -> Option<&Reservation> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// Whether a frame naming `id` may be accepted.
    pub fn is_referenceable(&self, id: u64) -> bool {
        self.get(id).is_some_and(Reservation::is_referenceable)
    }

    /// Declare a resource. The id is assigned here, not by the producer: an id
    /// the producer chose could collide with one already in the table, and the
    /// collision would be a frame naming the wrong texture.
    pub fn reserve(
        &mut self,
        total_bytes: u64,
        chunk_count: u32,
        format: u32,
        sha256: [u8; 32],
        deadline_nanos: u64,
    ) -> Result<u64, ResourceError> {
        if self.entries.len() >= MAX_RESERVATIONS {
            return Err(ResourceError::TooManyReservations);
        }
        if total_bytes == 0 || total_bytes > MAX_RESOURCE_BYTES {
            return Err(ResourceError::BadSize);
        }
        // The declared chunking has to be able to cover the declared size, or
        // the upload can never complete and the reservation sits until its
        // deadline holding its share of the table.
        if chunk_count == 0 || u64::from(chunk_count) * u64::from(MAX_CHUNK_BYTES) < total_bytes {
            return Err(ResourceError::BadChunkCount);
        }

        let id = self.next_id;
        self.next_id += 1;
        self.entries.push(Reservation {
            id,
            total_bytes,
            chunk_count,
            resource_epoch: self.epoch,
            format,
            sha256,
            deadline_nanos,
            state: ResourceState::Reserved,
            error: None,
            received_bytes: 0,
            next_chunk: 0,
        });
        Ok(id)
    }

    /// Accept one chunk's worth of bytes.
    ///
    /// The bytes are not held here -- the host writes them into its own staging
    /// buffer -- so this checks the bookkeeping that decides whether they may be
    /// written at all.
    pub fn accept_chunk(
        &mut self,
        id: u64,
        chunk_index: u32,
        byte_length: u32,
    ) -> Result<ResourceState, ResourceError> {
        let epoch = self.epoch;
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or(ResourceError::UnknownReservation)?;

        if entry.resource_epoch != epoch {
            return Err(ResourceError::EpochAdvanced);
        }
        if !matches!(
            entry.state,
            ResourceState::Reserved | ResourceState::Uploading
        ) {
            return Err(ResourceError::NotUploading);
        }
        // Strictly contiguous, for the same reason frame sequences are: a gap
        // means a chunk was lost, and there is no recovery from that which is
        // not "start the upload again".
        if chunk_index != entry.next_chunk {
            entry.state = ResourceState::Failed;
            entry.error = Some(ResourceError::NonContiguousChunk);
            return Err(ResourceError::NonContiguousChunk);
        }
        if byte_length == 0 || byte_length > MAX_CHUNK_BYTES {
            entry.state = ResourceState::Failed;
            entry.error = Some(ResourceError::ChunkOutOfBounds);
            return Err(ResourceError::ChunkOutOfBounds);
        }
        let received = entry.received_bytes + u64::from(byte_length);
        if received > entry.total_bytes || entry.next_chunk >= entry.chunk_count {
            entry.state = ResourceState::Failed;
            entry.error = Some(ResourceError::ChunkOutOfBounds);
            return Err(ResourceError::ChunkOutOfBounds);
        }

        entry.received_bytes = received;
        entry.next_chunk += 1;
        entry.state =
            if entry.received_bytes == entry.total_bytes && entry.next_chunk == entry.chunk_count {
                ResourceState::Verifying
            } else {
                ResourceState::Uploading
            };
        Ok(entry.state)
    }

    /// Compare the digest the host computed with the one the producer declared,
    /// and make the resource referenceable if they agree.
    pub fn verify(&mut self, id: u64, computed: [u8; 32]) -> Result<(), ResourceError> {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or(ResourceError::UnknownReservation)?;

        if entry.state != ResourceState::Verifying {
            // Either bytes are still missing, or this has already been settled.
            let error = if matches!(
                entry.state,
                ResourceState::Reserved | ResourceState::Uploading
            ) {
                ResourceError::Incomplete
            } else {
                ResourceError::NotUploading
            };
            entry.state = ResourceState::Failed;
            entry.error = Some(error);
            return Err(error);
        }
        // Constant-time comparison is not the concern here -- the digest is not
        // a secret and both sides know it -- but a short-circuit on the first
        // byte is, because it makes the failure depend on which byte differs.
        // `==` on a fixed array is a fixed-size compare either way.
        if computed != entry.sha256 {
            entry.state = ResourceState::Failed;
            entry.error = Some(ResourceError::DigestMismatch);
            return Err(ResourceError::DigestMismatch);
        }
        entry.state = ResourceState::Ready;
        entry.error = None;
        Ok(())
    }

    /// Fail every upload whose deadline has passed. Returns how many.
    pub fn expire(&mut self, now_nanos: u64) -> usize {
        let mut expired = 0;
        for entry in &mut self.entries {
            if matches!(
                entry.state,
                ResourceState::Reserved | ResourceState::Uploading | ResourceState::Verifying
            ) && now_nanos >= entry.deadline_nanos
            {
                entry.state = ResourceState::Failed;
                entry.error = Some(ResourceError::TimedOut);
                expired += 1;
            }
        }
        expired
    }

    /// Drop a reservation. The host releases whatever it staged for it.
    pub fn release(&mut self, id: u64) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        self.entries.len() != before
    }

    /// The GPU context was lost and the table rebuilt.
    ///
    /// Everything goes: the ids in the new table name different objects, so a
    /// resource that survived would be a name pointing at whatever took its
    /// place. Returns how many were discarded, which is what the host reports
    /// to the producer so it knows to upload them again.
    pub fn advance_epoch(&mut self, epoch: u64) -> Result<usize, ResourceError> {
        if epoch < self.epoch {
            return Err(ResourceError::EpochAdvanced);
        }
        let discarded = self.entries.len();
        self.entries.clear();
        self.epoch = epoch;
        Ok(discarded)
    }
}
