//! The v1 wire shape, pinned.
//!
//! Every struct here crosses the ABI boundary, so its size and the offset of
//! every field are part of the contract — not implementation detail a refactor
//! may change. A host compiled against v1 headers writes these exact bytes; a
//! library that lays them out differently reads one field's bits as another's
//! and reports success while doing it.
//!
//! These are `const` assertions rather than tests: a layout break should fail
//! the build of the crate that caused it, at the moment it is introduced,
//! rather than wait for a test run — and rather than wait for a v1 client to
//! meet the changed library in the field, which is when it would otherwise be
//! discovered.
//!
//! The size numbers below are LP64 (`linux-x86_64` and `aarch64-linux-android`,
//! the two targets that ship). ILP32 needs its own numbers and its own lane;
//! that part of the freeze blocker stays open until a 32-bit target exists,
//! because writing expected values for a target nobody builds would be
//! guesswork asserted as fact. The offset assertions that do not mention a
//! pointer hold on any target and are checked everywhere.

use std::mem::{align_of, offset_of, size_of};

use crate::{
    abi::VersionedHeader,
    callbacks::{MigoError, MigoHostCallbacks},
    input::{MigoTouchEvent, MigoTouchPoint},
    surface::{
        MigoAndroidNativeWindowDescriptor, MigoSurfaceDescriptor, MigoSurfaceMetrics,
        MigoX11WindowDescriptor,
    },
    MigoContentDescriptor, MigoEngineConfig, MigoSessionConfig,
};

/// Every versioned struct must begin with its header.
///
/// `validate_header` casts a caller's pointer to `*const VersionedHeader` and
/// reads `struct_size` before it trusts anything else. That cast is only sound
/// while the header is the first member: a struct that ever grew a field in
/// front of it would have its own bytes read as a size and a version, and the
/// validation that exists to catch a mismatched caller would be the thing
/// producing one.
macro_rules! header_is_first {
    ($($type:ty),+ $(,)?) => {$(
        const _: () = assert!(offset_of!($type, header) == 0);
        const _: () = assert!(align_of::<$type>() >= align_of::<VersionedHeader>());
    )+};
}

header_is_first!(
    MigoEngineConfig,
    MigoSessionConfig,
    MigoContentDescriptor,
    MigoHostCallbacks,
    MigoError,
    MigoSurfaceDescriptor,
    MigoSurfaceMetrics,
    MigoAndroidNativeWindowDescriptor,
    MigoX11WindowDescriptor,
    MigoTouchEvent,
);

// The header itself. Two `u32`s, in this order, on every target: it is the one
// shape a caller must get right before anything else can be negotiated.
const _: () = assert!(size_of::<VersionedHeader>() == 8);
const _: () = assert!(offset_of!(VersionedHeader, struct_size) == 0);
const _: () = assert!(offset_of!(VersionedHeader, abi_version) == 4);

// `MigoTouchPoint` has no header: it is an array element, not a versioned
// struct, and its count comes from the event that points at it. Its layout is
// load-bearing anyway — `to_touch_data` reinterprets an array of these as the
// engine's own point type — so every field is pinned here, on every target,
// since it contains no pointer.
const _: () = assert!(size_of::<MigoTouchPoint>() == 20);
const _: () = assert!(offset_of!(MigoTouchPoint, id) == 0);
const _: () = assert!(offset_of!(MigoTouchPoint, x) == 4);
const _: () = assert!(offset_of!(MigoTouchPoint, y) == 8);
const _: () = assert!(offset_of!(MigoTouchPoint, pressure) == 12);
const _: () = assert!(offset_of!(MigoTouchPoint, flags) == 16);

// Pointer-free structs: identical on ILP32 and LP64, so they are pinned for
// every target rather than only the ones that ship today.
const _: () = assert!(size_of::<MigoSessionConfig>() == 16);
const _: () = assert!(offset_of!(MigoSessionConfig, flags) == 8);

const _: () = assert!(size_of::<MigoSurfaceMetrics>() == 48);
const _: () = assert!(offset_of!(MigoSurfaceMetrics, generation) == 8);
const _: () = assert!(offset_of!(MigoSurfaceMetrics, width_pixels) == 16);
const _: () = assert!(offset_of!(MigoSurfaceMetrics, height_pixels) == 20);
const _: () = assert!(offset_of!(MigoSurfaceMetrics, scale_factor) == 24);
const _: () = assert!(offset_of!(MigoSurfaceMetrics, color_space) == 28);
const _: () = assert!(offset_of!(MigoSurfaceMetrics, alpha_mode) == 32);
const _: () = assert!(offset_of!(MigoSurfaceMetrics, preferred_presentation_mode) == 36);
const _: () = assert!(offset_of!(MigoSurfaceMetrics, flags) == 40);

/// LP64 sizes for the structs that contain a pointer.
///
/// Split out so the pointer-free assertions above still run on a 32-bit target
/// and only the numbers that genuinely depend on pointer width are skipped.
#[cfg(target_pointer_width = "64")]
mod lp64 {
    use super::*;

    const _: () = assert!(size_of::<MigoEngineConfig>() == 48);
    const _: () = assert!(offset_of!(MigoEngineConfig, flags) == 8);
    const _: () = assert!(offset_of!(MigoEngineConfig, reserved0) == 16);
    // A four-byte hole follows `reserved0`; the first pointer is aligned to 24.
    // The header must agree, which is why the hole is asserted rather than left
    // to be rediscovered by whoever adds a field into it.
    const _: () = assert!(offset_of!(MigoEngineConfig, files_dir_utf8) == 24);
    const _: () = assert!(offset_of!(MigoEngineConfig, cache_dir_utf8) == 32);
    const _: () = assert!(offset_of!(MigoEngineConfig, code_cache_dir_utf8) == 40);

    const _: () = assert!(size_of::<MigoContentDescriptor>() == 32);
    const _: () = assert!(offset_of!(MigoContentDescriptor, flags) == 8);
    const _: () = assert!(offset_of!(MigoContentDescriptor, reserved0) == 12);
    const _: () = assert!(offset_of!(MigoContentDescriptor, content_id_utf8) == 16);
    const _: () = assert!(offset_of!(MigoContentDescriptor, entry_utf8) == 24);

    const _: () = assert!(size_of::<MigoError>() == 32);
    const _: () = assert!(offset_of!(MigoError, code) == 8);
    const _: () = assert!(offset_of!(MigoError, flags) == 12);
    const _: () = assert!(offset_of!(MigoError, message_utf8) == 16);
    const _: () = assert!(offset_of!(MigoError, message_length) == 24);
    const _: () = assert!(offset_of!(MigoError, reserved0) == 28);

    // Function pointers are `Option<fn>` on the Rust side, which is a plain
    // nullable pointer with no discriminant. Pinning the offsets is what keeps
    // that niche optimisation from being an assumption.
    const _: () = assert!(size_of::<MigoHostCallbacks>() == 72);
    const _: () = assert!(offset_of!(MigoHostCallbacks, user_data) == 8);
    const _: () = assert!(offset_of!(MigoHostCallbacks, dispatcher_data) == 16);
    const _: () = assert!(offset_of!(MigoHostCallbacks, dispatch) == 24);
    const _: () = assert!(offset_of!(MigoHostCallbacks, on_ready) == 32);
    const _: () = assert!(offset_of!(MigoHostCallbacks, on_error) == 40);
    const _: () = assert!(offset_of!(MigoHostCallbacks, on_exit_requested) == 48);
    const _: () = assert!(offset_of!(MigoHostCallbacks, on_surface_lost) == 56);
    const _: () = assert!(offset_of!(MigoHostCallbacks, on_request_frame) == 64);

    const _: () = assert!(size_of::<MigoSurfaceDescriptor>() == 72);
    const _: () = assert!(offset_of!(MigoSurfaceDescriptor, generation) == 8);
    const _: () = assert!(offset_of!(MigoSurfaceDescriptor, platform_kind) == 16);
    const _: () = assert!(offset_of!(MigoSurfaceDescriptor, width_pixels) == 24);
    const _: () = assert!(offset_of!(MigoSurfaceDescriptor, scale_factor) == 32);
    // `capability_flags` is the first u64 after a run of u32s, so it pulls the
    // rest of the struct to 8-byte alignment; the two u32s that follow share a
    // slot and the pointer lands at 64.
    const _: () = assert!(offset_of!(MigoSurfaceDescriptor, capability_flags) == 48);
    const _: () = assert!(offset_of!(MigoSurfaceDescriptor, platform_descriptor_size) == 56);
    const _: () = assert!(offset_of!(MigoSurfaceDescriptor, platform_descriptor) == 64);

    const _: () = assert!(size_of::<MigoAndroidNativeWindowDescriptor>() == 24);
    const _: () = assert!(offset_of!(MigoAndroidNativeWindowDescriptor, native_window) == 16);

    const _: () = assert!(size_of::<MigoX11WindowDescriptor>() == 40);
    const _: () = assert!(offset_of!(MigoX11WindowDescriptor, display) == 16);
    const _: () = assert!(offset_of!(MigoX11WindowDescriptor, window) == 24);
    const _: () = assert!(offset_of!(MigoX11WindowDescriptor, screen) == 32);

    const _: () = assert!(size_of::<MigoTouchEvent>() == 32);
    const _: () = assert!(offset_of!(MigoTouchEvent, touch_type) == 8);
    const _: () = assert!(offset_of!(MigoTouchEvent, point_count) == 12);
    const _: () = assert!(offset_of!(MigoTouchEvent, timestamp_ms) == 16);
    const _: () = assert!(offset_of!(MigoTouchEvent, points) == 24);
}
