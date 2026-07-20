//! X11 and Wayland surface construction for desktop hosts.

use std::{
    ffi::{c_ulong, c_void},
    ptr::NonNull,
    sync::Arc,
};

use shared::surface::SurfaceRef;

use crate::{
    MIGO_PLATFORM_WAYLAND_SURFACE, MIGO_PLATFORM_X11_WINDOW,
    abi::{
        MIGO_ERROR_INTERNAL, MIGO_ERROR_INVALID_ARGUMENT, MIGO_ERROR_UNSUPPORTED_PLATFORM,
        MigoResult, VersionedHeader, validate_header,
    },
    surface::{MigoSurfaceDescriptor, MigoWaylandSurfaceDescriptor, MigoX11WindowDescriptor},
};

/// What a resize needs in order to rebuild the surface.
///
/// The values are copied rather than the descriptor pointer retained: the
/// header says `platform_descriptor` is borrowed for the attach call only, so
/// keeping it would outlive the caller's storage.
#[derive(Clone, Copy)]
pub(crate) enum PlatformTarget {
    X11 {
        window: c_ulong,
    },
    Wayland {
        surface: NonNull<c_void>,
        display: NonNull<c_void>,
    },
}

/// The surface kinds this build can attach, as a `MIGO_PLATFORM_*` bitmask.
///
/// `build_target` rejects by testing membership here rather than comparing
/// against its own constant, so what `migo_query_capabilities` advertises and
/// what an attach actually accepts cannot drift apart. A query that drifts is
/// worse than no query: a host would plan around an answer that is false.
pub(crate) const fn supported_platform_kinds() -> u64 {
    (1u64 << MIGO_PLATFORM_X11_WINDOW) | (1u64 << MIGO_PLATFORM_WAYLAND_SURFACE)
}

pub(crate) fn rebuild_surface(target: PlatformTarget, width: u32, height: u32) -> SurfaceRef {
    match target {
        PlatformTarget::X11 { window } => Arc::new(
            platform::desktop::presenter::LinuxX11Surface::new(window, width, height),
        ),
        PlatformTarget::Wayland { surface, .. } => Arc::new(
            platform::desktop::presenter::LinuxWaylandSurface::new(surface, width, height),
        ),
    }
}

/// Translate a validated descriptor into the engine's surface, graphics
/// platform, and the values a later resize needs.
///
/// # Safety
/// `descriptor` must have passed [`validate_header`].
pub(crate) unsafe fn build_target(
    descriptor: &MigoSurfaceDescriptor,
) -> Result<
    (
        SurfaceRef,
        graphics::egl_platform::GraphicsPlatform,
        PlatformTarget,
    ),
    MigoResult,
> {
    if !crate::platform::kind_is_supported(descriptor.platform_kind) {
        return Err(MIGO_ERROR_UNSUPPORTED_PLATFORM);
    }
    if descriptor.platform_kind == MIGO_PLATFORM_WAYLAND_SURFACE {
        return unsafe { build_wayland_target(descriptor) };
    }
    // The envelope's size field and the payload's own struct_size are an
    // intentional cross-check; disagreeing means the caller mismatched them.
    if descriptor.platform_descriptor_size as usize != size_of::<MigoX11WindowDescriptor>() {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    unsafe {
        validate_header(
            descriptor.platform_descriptor as *const VersionedHeader,
            size_of::<MigoX11WindowDescriptor>(),
        )
    }?;
    let x11 = unsafe { &*(descriptor.platform_descriptor as *const MigoX11WindowDescriptor) };
    if x11.platform_kind != MIGO_PLATFORM_X11_WINDOW {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    let Some(display) = NonNull::new(x11.display) else {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    };
    if x11.window == 0 || descriptor.width_pixels == 0 || descriptor.height_pixels == 0 {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }

    let window = x11.window as c_ulong;
    let surface: SurfaceRef = Arc::new(platform::desktop::presenter::LinuxX11Surface::new(
        window,
        descriptor.width_pixels,
        descriptor.height_pixels,
    ));
    let graphics_platform = platform::desktop::presenter::linux_x11_graphics_platform(display)
        .map_err(|error| {
            tracing::error!("build_target: graphics platform: {error:?}");
            MIGO_ERROR_INTERNAL
        })?;
    Ok((surface, graphics_platform, PlatformTarget::X11 { window }))
}

/// Translate a validated Wayland descriptor.
///
/// Split from `build_target` rather than folded into it: the two platforms
/// share only the envelope checks, and interleaving them made it easy to read
/// one platform's payload with the other's rules.
///
/// # Safety
/// `descriptor` must have passed [`validate_header`] and name the Wayland kind.
unsafe fn build_wayland_target(
    descriptor: &MigoSurfaceDescriptor,
) -> Result<
    (
        SurfaceRef,
        graphics::egl_platform::GraphicsPlatform,
        PlatformTarget,
    ),
    MigoResult,
> {
    if descriptor.platform_descriptor_size as usize != size_of::<MigoWaylandSurfaceDescriptor>() {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    unsafe {
        validate_header(
            descriptor.platform_descriptor as *const VersionedHeader,
            size_of::<MigoWaylandSurfaceDescriptor>(),
        )
    }?;
    let wayland =
        unsafe { &*(descriptor.platform_descriptor as *const MigoWaylandSurfaceDescriptor) };
    if wayland.platform_kind != MIGO_PLATFORM_WAYLAND_SURFACE {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    let (Some(display), Some(surface)) =
        (NonNull::new(wayland.display), NonNull::new(wayland.surface))
    else {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    };
    if descriptor.width_pixels == 0 || descriptor.height_pixels == 0 {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }

    let surface_ref: SurfaceRef = Arc::new(platform::desktop::presenter::LinuxWaylandSurface::new(
        surface,
        descriptor.width_pixels,
        descriptor.height_pixels,
    ));
    let graphics_platform = platform::desktop::presenter::linux_wayland_graphics_platform(display)
        .map_err(|error| {
            tracing::error!("build_wayland_target: graphics platform: {error:?}");
            MIGO_ERROR_INTERNAL
        })?;
    Ok((
        surface_ref,
        graphics_platform,
        PlatformTarget::Wayland { surface, display },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_UNSUPPORTED_PLATFORM};
    use std::ffi::c_void;

    fn wayland_descriptor(
        display: *mut c_void,
        surface: *mut c_void,
    ) -> MigoWaylandSurfaceDescriptor {
        MigoWaylandSurfaceDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoWaylandSurfaceDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            platform_kind: MIGO_PLATFORM_WAYLAND_SURFACE,
            flags: 0,
            display,
            surface,
        }
    }

    fn wayland_envelope(payload: &MigoWaylandSurfaceDescriptor) -> MigoSurfaceDescriptor {
        MigoSurfaceDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoSurfaceDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            generation: 1,
            platform_kind: MIGO_PLATFORM_WAYLAND_SURFACE,
            flags: 0,
            width_pixels: 720,
            height_pixels: 1280,
            scale_factor: 1.0,
            color_space: 0,
            alpha_mode: 0,
            preferred_presentation_mode: 0,
            capability_flags: 0,
            platform_descriptor_size: size_of::<MigoWaylandSurfaceDescriptor>() as u32,
            reserved0: 0,
            platform_descriptor: payload as *const _ as *const c_void,
        }
    }

    /// Both handles are required. A null one would reach EGL, which has no way
    /// to tell it apart from a display that simply has no surface yet.
    #[test]
    fn a_wayland_descriptor_requires_both_a_display_and_a_surface() {
        let no_display = wayland_descriptor(std::ptr::null_mut(), 0x5a5a_0001usize as *mut c_void);
        assert_eq!(
            unsafe { build_target(&wayland_envelope(&no_display)) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );

        let no_surface = wayland_descriptor(0xdead_beefusize as *mut c_void, std::ptr::null_mut());
        assert_eq!(
            unsafe { build_target(&wayland_envelope(&no_surface)) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    /// The envelope's size field and the payload's own must agree, the same
    /// cross-check X11 gets: a mismatch means the caller paired descriptors
    /// from different builds.
    #[test]
    fn a_wayland_descriptor_size_mismatch_is_rejected() {
        let payload = wayland_descriptor(
            0xdead_beefusize as *mut c_void,
            0x5a5a_0001usize as *mut c_void,
        );
        let mut envelope = wayland_envelope(&payload);
        envelope.platform_descriptor_size = 8;
        assert_eq!(
            unsafe { build_target(&envelope) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    /// A payload claiming a different kind than its envelope is a mismatched
    /// pair, not a Wayland surface -- reading it as one would hand EGL an X11
    /// display.
    #[test]
    fn a_wayland_envelope_wrapping_another_kind_is_rejected() {
        let mut payload = wayland_descriptor(
            0xdead_beefusize as *mut c_void,
            0x5a5a_0001usize as *mut c_void,
        );
        payload.platform_kind = MIGO_PLATFORM_X11_WINDOW;
        assert_eq!(
            unsafe { build_target(&wayland_envelope(&payload)) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    /// The capability query and the attach path must agree about Wayland, or a
    /// host plans around an answer that is false.
    #[test]
    fn wayland_is_advertised_as_supported() {
        assert!(crate::platform::kind_is_supported(
            MIGO_PLATFORM_WAYLAND_SURFACE
        ));
    }

    #[test]
    fn non_x11_platforms_are_reported_as_unsupported_not_invalid() {
        // A host on a platform this build does not implement should learn that,
        // rather than think its descriptor was malformed.
        let descriptor = MigoSurfaceDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoSurfaceDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            generation: 1,
            platform_kind: 2, // MIGO_PLATFORM_WIN32_HWND
            flags: 0,
            width_pixels: 640,
            height_pixels: 480,
            scale_factor: 1.0,
            color_space: 0,
            alpha_mode: 0,
            preferred_presentation_mode: 0,
            capability_flags: 0,
            platform_descriptor_size: size_of::<MigoX11WindowDescriptor>() as u32,
            reserved0: 0,
            platform_descriptor: std::ptr::null(),
        };
        let error = unsafe { build_target(&descriptor) }
            .err()
            .expect("rejected");
        assert_eq!(error, MIGO_ERROR_UNSUPPORTED_PLATFORM);
    }

    #[test]
    fn x11_descriptor_size_mismatch_is_rejected() {
        // The envelope's platform_descriptor_size and the payload's struct_size
        // are a deliberate cross-check; disagreement means a mismatched build.
        let x11 = MigoX11WindowDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoX11WindowDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            platform_kind: MIGO_PLATFORM_X11_WINDOW,
            flags: 0,
            display: 0xdead_beef_usize as *mut c_void,
            window: 0x2a0_0001,
            screen: 0,
            reserved0: 0,
        };
        let descriptor = MigoSurfaceDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoSurfaceDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            generation: 1,
            platform_kind: MIGO_PLATFORM_X11_WINDOW,
            flags: 0,
            width_pixels: 640,
            height_pixels: 480,
            scale_factor: 1.0,
            color_space: 0,
            alpha_mode: 0,
            preferred_presentation_mode: 0,
            capability_flags: 0,
            platform_descriptor_size: 8, // wrong on purpose
            reserved0: 0,
            platform_descriptor: &x11 as *const _ as *const c_void,
        };
        let error = unsafe { build_target(&descriptor) }
            .err()
            .expect("rejected");
        assert_eq!(error, MIGO_ERROR_INVALID_ARGUMENT);
    }

    #[test]
    fn x11_descriptor_requires_a_real_window_and_display() {
        let make = |display: *mut c_void, window: usize| MigoX11WindowDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoX11WindowDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            platform_kind: MIGO_PLATFORM_X11_WINDOW,
            flags: 0,
            display,
            window,
            screen: 0,
            reserved0: 0,
        };
        let envelope = |x11: &MigoX11WindowDescriptor| MigoSurfaceDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoSurfaceDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            generation: 1,
            platform_kind: MIGO_PLATFORM_X11_WINDOW,
            flags: 0,
            width_pixels: 640,
            height_pixels: 480,
            scale_factor: 1.0,
            color_space: 0,
            alpha_mode: 0,
            preferred_presentation_mode: 0,
            capability_flags: 0,
            platform_descriptor_size: size_of::<MigoX11WindowDescriptor>() as u32,
            reserved0: 0,
            platform_descriptor: x11 as *const _ as *const c_void,
        };

        let no_display = make(std::ptr::null_mut(), 0x2a0_0001);
        assert_eq!(
            unsafe { build_target(&envelope(&no_display)) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );

        let no_window = make(0xdead_beef_usize as *mut c_void, 0);
        assert_eq!(
            unsafe { build_target(&envelope(&no_window)) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }
}
