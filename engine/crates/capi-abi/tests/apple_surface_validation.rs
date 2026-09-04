//! The Apple platform payloads: macOS NSView/CAMetalLayer and iOS
//! UIView/CAMetalLayer.
//!
//! All four records are byte-identical in layout. They are four types, and
//! four `platform_kind` values, because the kind is the only thing that says
//! which ownership calls apply -- `retain`/`release` on a view versus a layer
//! whose `nextDrawable` the renderer will call. A single tagless `void*` would
//! let a host set the wrong kind, compile, parse cleanly, and only fail inside
//! Objective-C. These cases exist to keep that separation observable, so the
//! four types cannot be quietly collapsed back into one later.

use std::{ffi::c_void, mem::size_of, ptr::NonNull};

use migo_capi_abi::{
    MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_INVALID_ARGUMENT, MIGO_ERROR_UNSUPPORTED_PLATFORM,
    MigoResult, VersionedHeader,
    surface::{
        MIGO_ALPHA_MODE_OPAQUE, MIGO_COLOR_SPACE_SRGB, MIGO_PLATFORM_IOS_CA_METAL_LAYER,
        MIGO_PLATFORM_IOS_UI_VIEW, MIGO_PLATFORM_MACOS_CA_METAL_LAYER, MIGO_PLATFORM_MACOS_NS_VIEW,
        MIGO_PRESENTATION_MODE_DEFAULT, MigoIosMetalLayerDescriptor, MigoIosUiViewDescriptor,
        MigoMacosMetalLayerDescriptor, MigoMacosNsViewDescriptor, MigoSurfaceDescriptor,
        SurfaceDescriptorRef, ValidatedPlatformSurface,
    },
};

/// Every Apple kind, so a new one added to the enum without a case here shows
/// up as a compile error in `variant_for` rather than as silent coverage loss.
const APPLE_KINDS: [u32; 4] = [
    MIGO_PLATFORM_MACOS_NS_VIEW,
    MIGO_PLATFORM_MACOS_CA_METAL_LAYER,
    MIGO_PLATFORM_IOS_UI_VIEW,
    MIGO_PLATFORM_IOS_CA_METAL_LAYER,
];

fn header<T>() -> VersionedHeader {
    VersionedHeader {
        struct_size: size_of::<T>() as u32,
        abi_version: MIGO_ABI_VERSION_CURRENT,
    }
}

/// One Apple payload as raw bytes, built for `declared_kind` regardless of the
/// kind the envelope will announce. Keeping the two separable is the point:
/// several cases below deliberately disagree.
fn payload_bytes(
    declared_kind: u32,
    native: *mut c_void,
) -> [u8; size_of::<MigoIosUiViewDescriptor>()] {
    let record = MigoIosUiViewDescriptor {
        header: header::<MigoIosUiViewDescriptor>(),
        platform_kind: declared_kind,
        flags: 0,
        ui_view: native,
    };
    // SAFETY: the four Apple records share one layout (asserted below), and
    // this reads an initialised `#[repr(C)]` value as its own bytes.
    unsafe {
        std::mem::transmute::<MigoIosUiViewDescriptor, [u8; size_of::<MigoIosUiViewDescriptor>()]>(
            record,
        )
    }
}

fn descriptor(kind: u32, payload_size: u32, payload: *const c_void) -> MigoSurfaceDescriptor {
    MigoSurfaceDescriptor {
        header: header::<MigoSurfaceDescriptor>(),
        generation: 3,
        platform_kind: kind,
        flags: 0,
        width_pixels: 1170,
        height_pixels: 2532,
        scale_factor: 3.0,
        color_space: MIGO_COLOR_SPACE_SRGB,
        alpha_mode: MIGO_ALPHA_MODE_OPAQUE,
        preferred_presentation_mode: MIGO_PRESENTATION_MODE_DEFAULT,
        capability_flags: 0,
        platform_descriptor_size: payload_size,
        reserved0: 0,
        platform_descriptor: payload,
    }
}

/// Parse with only the kind under test enabled, so these cases keep testing
/// what they are named for after the implemented-kinds mask grows. The mask is
/// extended only once a device attach has succeeded; pinning to it here would
/// turn "iOS shipped" into "these tests changed meaning".
fn parse(
    kind: u32,
    payload_size: u32,
    payload: *const c_void,
) -> Result<SurfaceDescriptorRef, MigoResult> {
    let descriptor = descriptor(kind, payload_size, payload);
    // SAFETY: `payload` is a live, aligned Apple payload for `payload_size`
    // bytes, or null with a size that this call rejects before reading it.
    unsafe { SurfaceDescriptorRef::parse_for_platforms(&descriptor, 1u64 << kind) }
}

fn variant_for(kind: u32, native: NonNull<c_void>) -> ValidatedPlatformSurface {
    match kind {
        MIGO_PLATFORM_MACOS_NS_VIEW => ValidatedPlatformSurface::MacosNsView { ns_view: native },
        MIGO_PLATFORM_MACOS_CA_METAL_LAYER => ValidatedPlatformSurface::MacosMetalLayer {
            ca_metal_layer: native,
        },
        MIGO_PLATFORM_IOS_UI_VIEW => ValidatedPlatformSurface::IosUiView { ui_view: native },
        MIGO_PLATFORM_IOS_CA_METAL_LAYER => ValidatedPlatformSurface::IosMetalLayer {
            ca_metal_layer: native,
        },
        other => panic!("kind {other} is not an Apple kind"),
    }
}

#[test]
fn the_four_apple_records_share_one_layout_and_stay_four_types() {
    assert_eq!(
        size_of::<MigoMacosNsViewDescriptor>(),
        size_of::<MigoIosUiViewDescriptor>(),
    );
    assert_eq!(
        size_of::<MigoMacosMetalLayerDescriptor>(),
        size_of::<MigoIosMetalLayerDescriptor>(),
    );
    // Four distinct kind values. Equal layout is what makes a shared type
    // tempting; distinct kinds are what make it wrong.
    let mut seen = APPLE_KINDS;
    seen.sort_unstable();
    seen.windows(2)
        .for_each(|pair| assert_ne!(pair[0], pair[1], "Apple kinds must stay distinct"));
}

#[test]
fn each_apple_kind_parses_into_its_own_typed_variant() {
    let native = 0x1_0000_usize as *mut c_void;
    for kind in APPLE_KINDS {
        let bytes = payload_bytes(kind, native);
        let parsed = parse(kind, bytes.len() as u32, bytes.as_ptr().cast())
            .unwrap_or_else(|error| panic!("kind {kind} rejected: {error}"));
        assert_eq!(
            parsed.platform(),
            variant_for(kind, NonNull::new(native).unwrap()),
            "kind {kind} produced the wrong variant",
        );
    }
}

#[test]
fn an_envelope_kind_that_disagrees_with_its_payload_is_rejected() {
    // The exact confusion the four types exist to prevent: a host that
    // announces a CAMetalLayer attachment and hands over a UIView record.
    let native = 0x2_0000_usize as *mut c_void;
    for kind in APPLE_KINDS {
        for other in APPLE_KINDS {
            if other == kind {
                continue;
            }
            let bytes = payload_bytes(other, native);
            assert_eq!(
                parse(kind, bytes.len() as u32, bytes.as_ptr().cast()).unwrap_err(),
                MIGO_ERROR_INVALID_ARGUMENT,
                "envelope kind {kind} accepted a payload declaring {other}",
            );
        }
    }
}

#[test]
fn apple_native_identity_must_be_non_null() {
    for kind in APPLE_KINDS {
        let bytes = payload_bytes(kind, std::ptr::null_mut());
        assert_eq!(
            parse(kind, bytes.len() as u32, bytes.as_ptr().cast()).unwrap_err(),
            MIGO_ERROR_INVALID_ARGUMENT,
            "kind {kind} accepted a null native object",
        );
    }
}

#[test]
fn apple_payload_size_must_match_the_record_exactly() {
    let native = 0x3_0000_usize as *mut c_void;
    for kind in APPLE_KINDS {
        let bytes = payload_bytes(kind, native);
        for announced in [0u32, bytes.len() as u32 - 4, bytes.len() as u32 + 4] {
            assert_eq!(
                parse(kind, announced, bytes.as_ptr().cast()).unwrap_err(),
                MIGO_ERROR_INVALID_ARGUMENT,
                "kind {kind} accepted an announced size of {announced}",
            );
        }
    }
}

#[test]
fn apple_kinds_are_known_to_the_abi_but_gated_by_the_build() {
    // Two different rejections, and the difference is the whole point: an
    // unknown kind is a caller bug (INVALID_ARGUMENT), while a known kind this
    // binary cannot attach is a capability answer (UNSUPPORTED_PLATFORM). A
    // host probing for iOS support has to be able to tell those apart.
    let native = 0x4_0000_usize as *mut c_void;
    for kind in APPLE_KINDS {
        let bytes = payload_bytes(kind, native);
        let descriptor = descriptor(kind, bytes.len() as u32, bytes.as_ptr().cast());
        // SAFETY: `bytes` is a live, aligned payload of the announced size.
        let rejected = unsafe { SurfaceDescriptorRef::parse_for_platforms(&descriptor, 0) };
        assert_eq!(
            rejected.unwrap_err(),
            MIGO_ERROR_UNSUPPORTED_PLATFORM,
            "kind {kind} must report unsupported, not invalid, when not built in",
        );
    }

    let unknown = descriptor(11, 0, std::ptr::null());
    // SAFETY: the payload is never read for an unknown kind.
    assert_eq!(
        unsafe { SurfaceDescriptorRef::parse_for_platforms(&unknown, u64::MAX) }.unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT,
        "the first value above the Apple kinds must stay unknown",
    );
}
