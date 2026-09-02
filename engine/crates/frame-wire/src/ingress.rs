//! The single door frames come in through, and the credit accounting behind it.
//!
//! One entry point, deliberately. The C ABI exposes exactly one function that
//! accepts frame bytes; a second path -- a debug shortcut, a "fast" variant --
//! would be a second place for the nonce, generation and sequence rules to be
//! almost right.

use crate::{WireError, WireFrame, validate};

/// What the host should do with the packet it just handed over.
///
/// The numbers cross the C ABI. Never renumber; only append.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum IngressDecision {
    /// Taken, and a credit consumed until the renderer reports completion.
    Accepted = 1,
    /// Legal, but there is no credit. The producer must wait, not retry
    /// immediately and not drop the packet: it may carry state or resource
    /// changes that a later frame depends on.
    WouldBlock = 2,
    /// Malformed, or not addressed to this session. Costs no credit, and the
    /// producer must not resend the same bytes.
    Rejected = 3,
    /// Correct bytes for a runtime generation that no longer exists -- the
    /// WebContent process was replaced, or the session was reloaded. Not an
    /// error on anyone's part, and not something a retry fixes.
    GenerationLost = 4,
}

/// The full answer, including what the producer needs to schedule the next one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IngressOutcome {
    pub decision: IngressDecision,
    pub remaining_credits: u32,
    /// Non-zero only for [`IngressDecision::Accepted`].
    pub accepted_sequence: u64,
    /// Non-zero only for [`IngressDecision::Rejected`], and stable across
    /// releases so production telemetry can tell the failures apart.
    pub wire_error_code: u32,
}

impl IngressOutcome {
    fn rejected(error: WireError, remaining_credits: u32) -> Self {
        Self {
            decision: IngressDecision::Rejected,
            remaining_credits,
            accepted_sequence: 0,
            wire_error_code: error.code(),
        }
    }
}

/// Reasons a packet is rejected that are not about its bytes.
///
/// Numbered above [`WireError`]'s range so a single telemetry field can carry
/// either without ambiguity.
pub const INGRESS_ERROR_FOREIGN_SESSION: u32 = 1001;
pub const INGRESS_ERROR_STALE_SEQUENCE: u32 = 1002;
pub const INGRESS_ERROR_STALE_SURFACE: u32 = 1003;
pub const INGRESS_ERROR_STALE_RESOURCE_EPOCH: u32 = 1004;

/// Default credit depth.
///
/// Two, so the producer can be building frame N+1 while the renderer works on
/// N, and no deeper: every additional credit is another frame of input latency
/// and another packet's worth of memory in flight. The value is tunable because
/// the right depth is a measurement, not a constant -- but it changes on device
/// data, not on a hunch.
pub const DEFAULT_MAX_CREDITS: u32 = 2;

/// Accepts frames for one runtime generation of one session.
///
/// A new generation gets a new `FrameIngress`. Nothing here is reset in place:
/// resetting would mean a packet in flight from the old generation could be
/// accepted by the new one, which is exactly the confusion generations exist to
/// prevent.
#[derive(Debug)]
pub struct FrameIngress {
    session_nonce: u64,
    runtime_generation: u32,
    surface_generation: u32,
    resource_epoch: u32,
    max_credits: u32,
    in_flight: u32,
    last_accepted_sequence: u64,
}

impl FrameIngress {
    pub fn new(session_nonce: u64, runtime_generation: u32) -> Self {
        Self {
            session_nonce,
            runtime_generation,
            surface_generation: 0,
            resource_epoch: 0,
            max_credits: DEFAULT_MAX_CREDITS,
            in_flight: 0,
            last_accepted_sequence: 0,
        }
    }

    pub fn with_max_credits(mut self, credits: u32) -> Self {
        self.max_credits = credits.max(1);
        self
    }

    /// Advance to a new surface. Packets addressed to an older surface
    /// generation are rejected from here on: they were built against a size,
    /// scale or colour space that no longer describes what will be presented.
    pub fn set_surface_generation(&mut self, generation: u32) {
        self.surface_generation = generation;
    }

    /// Advance the resource epoch, invalidating every resource id a producer
    /// may still be holding. Used when the GPU context is lost and the resource
    /// table is rebuilt: ids are reused, so without an epoch a stale id from
    /// before the loss silently names a different object.
    pub fn set_resource_epoch(&mut self, epoch: u32) {
        self.resource_epoch = epoch;
    }

    #[inline]
    pub const fn remaining_credits(&self) -> u32 {
        self.max_credits - self.in_flight
    }

    #[inline]
    pub const fn in_flight(&self) -> u32 {
        self.in_flight
    }

    #[inline]
    pub const fn last_accepted_sequence(&self) -> u64 {
        self.last_accepted_sequence
    }

    /// Offer one packet.
    ///
    /// Validation runs before the credit check, and that order is deliberate.
    /// It costs a CRC pass on a packet that may be told to wait, and it buys
    /// the property that whether a packet is *legal* never depends on how busy
    /// the renderer is. The alternative answers `WouldBlock` to malformed bytes
    /// and invites the producer to resend them forever.
    pub fn submit<'a>(&mut self, bytes: &'a [u8]) -> (IngressOutcome, Option<WireFrame<'a>>) {
        let frame = match validate(bytes) {
            Ok(frame) => frame,
            Err(error) => {
                return (
                    IngressOutcome::rejected(error, self.remaining_credits()),
                    None,
                );
            }
        };

        if frame.session_nonce() != self.session_nonce {
            return (
                IngressOutcome {
                    decision: IngressDecision::Rejected,
                    remaining_credits: self.remaining_credits(),
                    accepted_sequence: 0,
                    wire_error_code: INGRESS_ERROR_FOREIGN_SESSION,
                },
                None,
            );
        }

        // Generation before sequence: a packet from a dead generation is not a
        // sequencing mistake, and reporting it as one would send the producer
        // looking for a bug in its own counter.
        if frame.runtime_generation() != self.runtime_generation {
            return (
                IngressOutcome {
                    decision: IngressDecision::GenerationLost,
                    remaining_credits: self.remaining_credits(),
                    accepted_sequence: 0,
                    wire_error_code: 0,
                },
                None,
            );
        }

        if self.surface_generation != 0 && frame.surface_generation() != self.surface_generation {
            return (
                IngressOutcome {
                    decision: IngressDecision::Rejected,
                    remaining_credits: self.remaining_credits(),
                    accepted_sequence: 0,
                    wire_error_code: INGRESS_ERROR_STALE_SURFACE,
                },
                None,
            );
        }

        if frame.resource_epoch() != self.resource_epoch {
            return (
                IngressOutcome {
                    decision: IngressDecision::Rejected,
                    remaining_credits: self.remaining_credits(),
                    accepted_sequence: 0,
                    wire_error_code: INGRESS_ERROR_STALE_RESOURCE_EPOCH,
                },
                None,
            );
        }

        // Strictly increasing. A repeat is a replay and a decrease is a
        // reorder; both would draw a frame twice or out of order, and neither
        // is recoverable by the consumer.
        if frame.sequence() <= self.last_accepted_sequence {
            return (
                IngressOutcome {
                    decision: IngressDecision::Rejected,
                    remaining_credits: self.remaining_credits(),
                    accepted_sequence: 0,
                    wire_error_code: INGRESS_ERROR_STALE_SEQUENCE,
                },
                None,
            );
        }

        if self.in_flight >= self.max_credits {
            return (
                IngressOutcome {
                    decision: IngressDecision::WouldBlock,
                    remaining_credits: 0,
                    accepted_sequence: 0,
                    wire_error_code: 0,
                },
                None,
            );
        }

        self.in_flight += 1;
        self.last_accepted_sequence = frame.sequence();
        (
            IngressOutcome {
                decision: IngressDecision::Accepted,
                remaining_credits: self.remaining_credits(),
                accepted_sequence: frame.sequence(),
                wire_error_code: 0,
            },
            Some(frame),
        )
    }

    /// The renderer finished with an accepted packet; return its credit.
    ///
    /// Saturating rather than wrapping: a double completion is a bug in the
    /// renderer, and the useful failure is a stalled producer someone
    /// investigates, not a credit counter that wraps to four billion and turns
    /// backpressure off.
    pub fn complete(&mut self) {
        self.in_flight = self.in_flight.saturating_sub(1);
    }
}
