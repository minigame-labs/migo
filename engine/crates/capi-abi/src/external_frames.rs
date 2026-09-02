//! The ABI record a host reads back after offering one externally produced
//! frame.
//!
//! This module carries no entry point, matching `include/migo/external_frames.h`.
//! The layout is frozen now because the Swift transport is written against it,
//! and the submit function lands with an implementation behind it. An exported
//! symbol that always fails is what shipped a Windows SDK that loaded, resolved
//! every entry point, and could attach nothing.

use std::mem::{offset_of, size_of};

use crate::{
    AbiStruct, MIGO_ERROR_INVALID_ARGUMENT, MIGO_OK, MigoResult, OutputVersionPolicy,
    VersionedHeader, write_versioned_output,
};

pub const MIGO_FRAME_INGRESS_ACCEPTED: u32 = 1;
pub const MIGO_FRAME_INGRESS_WOULD_BLOCK: u32 = 2;
pub const MIGO_FRAME_INGRESS_REJECTED: u32 = 3;
pub const MIGO_FRAME_INGRESS_GENERATION_LOST: u32 = 4;

/// One ingress answer.
///
/// `accepted_sequence` is placed before the 32-bit fields so the record is 32
/// bytes with no interior padding on both LP64 and ILP32 -- one layout, rather
/// than two that happen to agree today.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MigoFrameIngressOutcome {
    pub header: VersionedHeader,
    pub accepted_sequence: u64,
    pub decision: u32,
    pub remaining_credits: u32,
    pub wire_error_code: u32,
    pub reserved0: u32,
}

// SAFETY: every field has an all-zero representation and v1 requires the
// complete record.
unsafe impl AbiStruct for MigoFrameIngressOutcome {}

/// Write one ingress answer without exposing Rust layout or padding.
///
/// The consistency rules are enforced here rather than trusted from the caller,
/// because they are what the host's own logic keys on. A `REJECTED` carrying a
/// sequence number, or an `ACCEPTED` carrying an error code, would each be a
/// contradiction the host has no way to resolve -- and the natural bug is to
/// forget to clear one field on an early return.
///
/// # Safety
/// `out` must satisfy [`write_versioned_output`]'s output contract.
pub unsafe fn write_frame_ingress_outcome(
    out: *mut MigoFrameIngressOutcome,
    decision: u32,
    remaining_credits: u32,
    accepted_sequence: u64,
    wire_error_code: u32,
) -> MigoResult {
    let recognised = matches!(
        decision,
        MIGO_FRAME_INGRESS_ACCEPTED
            | MIGO_FRAME_INGRESS_WOULD_BLOCK
            | MIGO_FRAME_INGRESS_REJECTED
            | MIGO_FRAME_INGRESS_GENERATION_LOST
    );
    if !recognised {
        return MIGO_ERROR_INVALID_ARGUMENT;
    }
    // A sequence number means "this packet was taken". Only ACCEPTED took one.
    if (decision == MIGO_FRAME_INGRESS_ACCEPTED) != (accepted_sequence != 0) {
        return MIGO_ERROR_INVALID_ARGUMENT;
    }
    // And an error code means "these bytes were refused". Only REJECTED refuses:
    // GENERATION_LOST is not a fault, and WOULD_BLOCK is not a verdict on the
    // bytes at all.
    if (decision == MIGO_FRAME_INGRESS_REJECTED) != (wire_error_code != 0) {
        return MIGO_ERROR_INVALID_ARGUMENT;
    }
    // Backpressure is level-triggered: WOULD_BLOCK is the answer precisely when
    // no credit remains, so a WOULD_BLOCK advertising credit would tell the
    // producer to retry immediately into the same refusal.
    if (decision == MIGO_FRAME_INGRESS_WOULD_BLOCK) && remaining_credits != 0 {
        return MIGO_ERROR_INVALID_ARGUMENT;
    }

    let value = MigoFrameIngressOutcome {
        header: VersionedHeader {
            struct_size: size_of::<MigoFrameIngressOutcome>() as u32,
            abi_version: crate::MIGO_ABI_VERSION_CURRENT,
        },
        accepted_sequence,
        decision,
        remaining_credits,
        wire_error_code,
        reserved0: 0,
    };

    // SAFETY: forwarded from this function's contract; `value` is a distinct
    // local, fully initialized ABI record.
    let result = unsafe { write_versioned_output(out, &value, OutputVersionPolicy::CurrentAbi) };
    debug_assert!(result != MIGO_OK || !out.is_null());
    result
}

const _: () = assert!(size_of::<MigoFrameIngressOutcome>() == 32);
const _: () = assert!(offset_of!(MigoFrameIngressOutcome, header) == 0);
const _: () = assert!(offset_of!(MigoFrameIngressOutcome, accepted_sequence) == 8);
const _: () = assert!(offset_of!(MigoFrameIngressOutcome, decision) == 16);
const _: () = assert!(offset_of!(MigoFrameIngressOutcome, remaining_credits) == 20);
const _: () = assert!(offset_of!(MigoFrameIngressOutcome, wire_error_code) == 24);
const _: () = assert!(offset_of!(MigoFrameIngressOutcome, reserved0) == 28);
