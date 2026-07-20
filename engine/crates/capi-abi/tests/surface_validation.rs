use std::{ffi::c_void, mem::size_of};

use migo_capi_abi::{
    MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_INVALID_ARGUMENT, MIGO_ERROR_UNSUPPORTED_ABI,
    MIGO_ERROR_UNSUPPORTED_CAPABILITY, MIGO_ERROR_UNSUPPORTED_PLATFORM, VersionedHeader,
    surface::{
        MIGO_ALPHA_MODE_OPAQUE, MIGO_ALPHA_MODE_POSTMULTIPLIED, MIGO_ALPHA_MODE_PREMULTIPLIED,
        MIGO_COLOR_SPACE_DISPLAY_P3, MIGO_COLOR_SPACE_EXTENDED_SRGB, MIGO_COLOR_SPACE_SRGB,
        MIGO_PLATFORM_ANDROID_NATIVE_WINDOW, MIGO_PLATFORM_WAYLAND_SURFACE,
        MIGO_PLATFORM_WIN32_HWND, MIGO_PLATFORM_X11_WINDOW, MIGO_PRESENTATION_MODE_DEFAULT,
        MIGO_PRESENTATION_MODE_FIFO, MIGO_PRESENTATION_MODE_IMMEDIATE,
        MIGO_PRESENTATION_MODE_MAILBOX, MIGO_SURFACE_CAPABILITY_TRANSPARENT,
        MIGO_SURFACE_CAPABILITY_WIDE_COLOR, MigoAndroidNativeWindowDescriptor,
        MigoSurfaceDescriptor, MigoSurfaceMetrics, MigoWaylandSurfaceDescriptor,
        MigoX11WindowDescriptor, SurfaceDescriptorRef, ValidatedPlatformSurface,
        validate_attach_generation, validate_update_generation,
    },
};

#[test]
fn public_attachment_generations_are_strictly_monotonic_per_session() {
    assert_eq!(validate_attach_generation(1, 0), Ok(1));
    assert_eq!(validate_attach_generation(9, 1), Ok(9));
    assert_eq!(
        validate_attach_generation(9, 9),
        Err(migo_capi_abi::MIGO_ERROR_STALE_SURFACE),
    );
    assert_eq!(
        validate_attach_generation(8, 9),
        Err(migo_capi_abi::MIGO_ERROR_STALE_SURFACE),
    );
    assert_eq!(
        validate_attach_generation(0, 9),
        Err(MIGO_ERROR_INVALID_ARGUMENT),
    );
}

#[test]
fn updates_require_the_exact_active_public_generation() {
    assert_eq!(validate_update_generation(9, 9), Ok(()));
    assert_eq!(
        validate_update_generation(8, 9),
        Err(migo_capi_abi::MIGO_ERROR_STALE_SURFACE),
    );
    assert_eq!(
        validate_update_generation(10, 9),
        Err(MIGO_ERROR_INVALID_ARGUMENT),
    );
    assert_eq!(
        validate_update_generation(0, 9),
        Err(MIGO_ERROR_INVALID_ARGUMENT),
    );
}

fn header<T>() -> VersionedHeader {
    VersionedHeader {
        struct_size: size_of::<T>() as u32,
        abi_version: MIGO_ABI_VERSION_CURRENT,
    }
}

fn wayland(display: *mut c_void, surface: *mut c_void) -> MigoWaylandSurfaceDescriptor {
    MigoWaylandSurfaceDescriptor {
        header: header::<MigoWaylandSurfaceDescriptor>(),
        platform_kind: MIGO_PLATFORM_WAYLAND_SURFACE,
        flags: 0,
        display,
        surface,
    }
}

fn descriptor(kind: u32, payload_size: u32, payload: *const c_void) -> MigoSurfaceDescriptor {
    MigoSurfaceDescriptor {
        header: header::<MigoSurfaceDescriptor>(),
        generation: 7,
        platform_kind: kind,
        flags: 0,
        width_pixels: 1280,
        height_pixels: 720,
        scale_factor: 2.0,
        color_space: MIGO_COLOR_SPACE_SRGB,
        alpha_mode: MIGO_ALPHA_MODE_OPAQUE,
        preferred_presentation_mode: MIGO_PRESENTATION_MODE_DEFAULT,
        capability_flags: 0,
        platform_descriptor_size: payload_size,
        reserved0: 0,
        platform_descriptor: payload,
    }
}

fn valid_wayland() -> (MigoWaylandSurfaceDescriptor, MigoSurfaceDescriptor) {
    let payload = wayland(
        0xdead_beefusize as *mut c_void,
        0x5a5a_0001usize as *mut c_void,
    );
    let envelope = descriptor(
        MIGO_PLATFORM_WAYLAND_SURFACE,
        size_of::<MigoWaylandSurfaceDescriptor>() as u32,
        std::ptr::null(),
    );
    (payload, envelope)
}

fn parse(descriptor: &MigoSurfaceDescriptor) -> Result<SurfaceDescriptorRef, i32> {
    unsafe { SurfaceDescriptorRef::parse(descriptor) }
}

fn parse_with_payload<T>(
    descriptor: &MigoSurfaceDescriptor,
    payload: &T,
) -> Result<SurfaceDescriptorRef, i32> {
    let mut copied = *descriptor;
    copied.platform_descriptor = (payload as *const T).cast();
    parse(&copied)
}

#[test]
fn a_valid_surface_is_copied_into_a_typed_configuration() {
    let (payload, descriptor) = valid_wayland();

    let validated = parse_with_payload(&descriptor, &payload).expect("valid Wayland surface");
    assert_eq!(validated.public_generation(), 7);
    assert_eq!(validated.configuration().width_pixels(), 1280);
    assert_eq!(validated.configuration().height_pixels(), 720);
    assert_eq!(validated.configuration().scale_factor(), 2.0);
    assert!(matches!(
        validated.platform(),
        ValidatedPlatformSurface::Wayland { .. },
    ));
}

#[test]
fn generation_dimensions_and_scale_are_strictly_validated() {
    let (payload, mut descriptor) = valid_wayland();

    descriptor.generation = 0;
    assert_eq!(
        parse_with_payload(&descriptor, &payload).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
    descriptor.generation = 7;

    descriptor.width_pixels = 0;
    assert_eq!(
        parse_with_payload(&descriptor, &payload).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
    descriptor.width_pixels = 1280;
    descriptor.height_pixels = 0;
    assert_eq!(
        parse_with_payload(&descriptor, &payload).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
    descriptor.height_pixels = 720;

    descriptor.width_pixels = i32::MAX as u32 + 1;
    assert_eq!(
        parse_with_payload(&descriptor, &payload).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
    descriptor.width_pixels = 1280;
    descriptor.height_pixels = i32::MAX as u32 + 1;
    assert_eq!(
        parse_with_payload(&descriptor, &payload).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
    descriptor.height_pixels = 720;

    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, -f32::INFINITY] {
        descriptor.scale_factor = scale;
        assert_eq!(
            parse_with_payload(&descriptor, &payload).unwrap_err(),
            MIGO_ERROR_INVALID_ARGUMENT
        );
    }
}

#[test]
fn supported_defaults_and_fifo_are_accepted() {
    let (payload, mut descriptor) = valid_wayland();

    for presentation in [MIGO_PRESENTATION_MODE_DEFAULT, MIGO_PRESENTATION_MODE_FIFO] {
        descriptor.preferred_presentation_mode = presentation;
        parse_with_payload(&descriptor, &payload).expect("supported presentation mode");
    }
}

#[test]
fn known_but_unimplemented_color_alpha_and_presentation_modes_are_unsupported() {
    let (payload, mut descriptor) = valid_wayland();

    for color in [MIGO_COLOR_SPACE_DISPLAY_P3, MIGO_COLOR_SPACE_EXTENDED_SRGB] {
        descriptor.color_space = color;
        assert_eq!(
            parse_with_payload(&descriptor, &payload).unwrap_err(),
            MIGO_ERROR_UNSUPPORTED_CAPABILITY,
        );
    }
    descriptor.color_space = MIGO_COLOR_SPACE_SRGB;

    for alpha in [
        MIGO_ALPHA_MODE_PREMULTIPLIED,
        MIGO_ALPHA_MODE_POSTMULTIPLIED,
    ] {
        descriptor.alpha_mode = alpha;
        assert_eq!(
            parse_with_payload(&descriptor, &payload).unwrap_err(),
            MIGO_ERROR_UNSUPPORTED_CAPABILITY,
        );
    }
    descriptor.alpha_mode = MIGO_ALPHA_MODE_OPAQUE;

    for presentation in [
        MIGO_PRESENTATION_MODE_MAILBOX,
        MIGO_PRESENTATION_MODE_IMMEDIATE,
    ] {
        descriptor.preferred_presentation_mode = presentation;
        assert_eq!(
            parse_with_payload(&descriptor, &payload).unwrap_err(),
            MIGO_ERROR_UNSUPPORTED_CAPABILITY,
        );
    }
}

#[test]
fn unknown_enums_flags_and_reserved_values_are_invalid() {
    let (payload, mut descriptor) = valid_wayland();

    descriptor.color_space = 99;
    assert_eq!(
        parse_with_payload(&descriptor, &payload).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
    descriptor.color_space = MIGO_COLOR_SPACE_SRGB;
    descriptor.alpha_mode = 99;
    assert_eq!(
        parse_with_payload(&descriptor, &payload).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
    descriptor.alpha_mode = MIGO_ALPHA_MODE_OPAQUE;
    descriptor.preferred_presentation_mode = 99;
    assert_eq!(
        parse_with_payload(&descriptor, &payload).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
    descriptor.preferred_presentation_mode = MIGO_PRESENTATION_MODE_DEFAULT;
    descriptor.flags = 1;
    assert_eq!(
        parse_with_payload(&descriptor, &payload).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
    descriptor.flags = 0;
    descriptor.reserved0 = 1;
    assert_eq!(
        parse_with_payload(&descriptor, &payload).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
}

#[test]
fn required_capabilities_never_silently_fall_back() {
    let (payload, mut descriptor) = valid_wayland();

    for capability in [
        MIGO_SURFACE_CAPABILITY_WIDE_COLOR,
        MIGO_SURFACE_CAPABILITY_TRANSPARENT,
    ] {
        descriptor.capability_flags = capability;
        assert_eq!(
            parse_with_payload(&descriptor, &payload).unwrap_err(),
            MIGO_ERROR_UNSUPPORTED_CAPABILITY,
        );
    }
    descriptor.capability_flags = 1 << 63;
    assert_eq!(
        parse_with_payload(&descriptor, &payload).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
}

#[test]
fn envelope_and_typed_payload_size_header_kind_and_flags_must_agree() {
    let (mut payload, mut descriptor) = valid_wayland();

    descriptor.platform_descriptor_size = 8;
    assert_eq!(
        parse_with_payload(&descriptor, &payload).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
    descriptor.platform_descriptor_size = size_of::<MigoWaylandSurfaceDescriptor>() as u32;

    payload.header.struct_size = 8;
    assert_eq!(
        parse_with_payload(&descriptor, &payload).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
    payload.header = header::<MigoWaylandSurfaceDescriptor>();
    payload.header.abi_version += 1;
    assert_eq!(
        parse_with_payload(&descriptor, &payload).unwrap_err(),
        MIGO_ERROR_UNSUPPORTED_ABI
    );
    payload.header = header::<MigoWaylandSurfaceDescriptor>();

    payload.platform_kind = MIGO_PLATFORM_X11_WINDOW;
    assert_eq!(
        parse_with_payload(&descriptor, &payload).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
    payload.platform_kind = MIGO_PLATFORM_WAYLAND_SURFACE;
    payload.flags = 1;
    assert_eq!(
        parse_with_payload(&descriptor, &payload).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
}

#[test]
fn x11_wayland_and_android_native_identity_must_be_non_null() {
    let mut x11 = MigoX11WindowDescriptor {
        header: header::<MigoX11WindowDescriptor>(),
        platform_kind: MIGO_PLATFORM_X11_WINDOW,
        flags: 0,
        display: std::ptr::null_mut(),
        window: 1,
        screen: 0,
        reserved0: 0,
    };
    let mut envelope = descriptor(
        MIGO_PLATFORM_X11_WINDOW,
        size_of::<MigoX11WindowDescriptor>() as u32,
        std::ptr::null(),
    );
    assert_eq!(
        parse_with_payload(&envelope, &x11).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
    x11.display = 0xdead_beefusize as *mut c_void;
    x11.window = 0;
    assert_eq!(
        parse_with_payload(&envelope, &x11).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );

    let mut wayland = wayland(std::ptr::null_mut(), 0x5a5a_0001usize as *mut c_void);
    envelope = descriptor(
        MIGO_PLATFORM_WAYLAND_SURFACE,
        size_of::<MigoWaylandSurfaceDescriptor>() as u32,
        std::ptr::null(),
    );
    assert_eq!(
        parse_with_payload(&envelope, &wayland).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
    wayland.display = 0xdead_beefusize as *mut c_void;
    wayland.surface = std::ptr::null_mut();
    assert_eq!(
        parse_with_payload(&envelope, &wayland).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );

    let android = MigoAndroidNativeWindowDescriptor {
        header: header::<MigoAndroidNativeWindowDescriptor>(),
        platform_kind: MIGO_PLATFORM_ANDROID_NATIVE_WINDOW,
        flags: 0,
        native_window: std::ptr::null_mut(),
    };
    envelope = descriptor(
        MIGO_PLATFORM_ANDROID_NATIVE_WINDOW,
        size_of::<MigoAndroidNativeWindowDescriptor>() as u32,
        std::ptr::null(),
    );
    assert_eq!(
        parse_with_payload(&envelope, &android).unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT
    );
}

#[test]
fn unsupported_platforms_are_rejected_before_their_payload_is_touched() {
    let unsupported_descriptor = descriptor(MIGO_PLATFORM_X11_WINDOW, 0, std::ptr::null());
    let only_wayland = 1u64 << MIGO_PLATFORM_WAYLAND_SURFACE;
    assert_eq!(
        unsafe { SurfaceDescriptorRef::parse_for_platforms(&unsupported_descriptor, only_wayland) }
            .unwrap_err(),
        MIGO_ERROR_UNSUPPORTED_PLATFORM,
    );

    let known_but_unimplemented = descriptor(MIGO_PLATFORM_WIN32_HWND, 0, std::ptr::null());
    assert_eq!(
        unsafe { SurfaceDescriptorRef::parse(&known_but_unimplemented) }.unwrap_err(),
        MIGO_ERROR_UNSUPPORTED_PLATFORM,
    );

    let unknown = descriptor(99, 0, std::ptr::null());
    assert_eq!(
        unsafe { SurfaceDescriptorRef::parse(&unknown) }.unwrap_err(),
        MIGO_ERROR_INVALID_ARGUMENT,
    );
}

#[test]
fn metrics_use_the_same_configuration_rules() {
    let mut metrics = MigoSurfaceMetrics {
        header: header::<MigoSurfaceMetrics>(),
        generation: 7,
        width_pixels: 800,
        height_pixels: 600,
        scale_factor: 1.5,
        color_space: MIGO_COLOR_SPACE_SRGB,
        alpha_mode: MIGO_ALPHA_MODE_OPAQUE,
        preferred_presentation_mode: MIGO_PRESENTATION_MODE_FIFO,
        flags: 0,
        reserved0: 0,
    };
    let validated = metrics.validate().expect("valid metrics");
    assert_eq!(validated.public_generation(), 7);
    assert_eq!(validated.configuration().scale_factor(), 1.5);

    metrics.generation = 0;
    assert_eq!(metrics.validate().unwrap_err(), MIGO_ERROR_INVALID_ARGUMENT,);
}
