//! X11 surface construction for desktop hosts.

use std::{ffi::c_ulong, ptr::NonNull, sync::Arc};

use shared::surface::SurfaceRef;

use crate::{
    abi::{
        validate_header, MigoResult, VersionedHeader, MIGO_ERROR_INTERNAL,
        MIGO_ERROR_INVALID_ARGUMENT, MIGO_ERROR_UNSUPPORTED_PLATFORM,
    },
    surface::{MigoSurfaceDescriptor, MigoX11WindowDescriptor},
    MIGO_PLATFORM_X11_WINDOW,
};

/// What a resize needs in order to rebuild the surface.
///
/// The values are copied rather than the descriptor pointer retained: the
/// header says `platform_descriptor` is borrowed for the attach call only, so
/// keeping it would outlive the caller's storage.
#[derive(Clone, Copy)]
pub(crate) enum PlatformTarget {
    X11 { window: c_ulong },
}

pub(crate) fn rebuild_surface(target: PlatformTarget, width: u32, height: u32) -> SurfaceRef {
    match target {
        PlatformTarget::X11 { window } => Arc::new(
            platform::desktop::presenter::LinuxX11Surface::new(window, width, height),
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
    if descriptor.platform_kind != MIGO_PLATFORM_X11_WINDOW {
        return Err(MIGO_ERROR_UNSUPPORTED_PLATFORM);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_UNSUPPORTED_PLATFORM};
    use std::ffi::c_void;

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
        let error = unsafe { build_target(&descriptor) }.err().expect("rejected");
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
        let error = unsafe { build_target(&descriptor) }.err().expect("rejected");
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
