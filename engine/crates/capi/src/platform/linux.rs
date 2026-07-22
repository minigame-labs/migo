//! X11 and Wayland surface construction for Linux hosts.

use std::{
    ffi::{c_ulong, c_void},
    ptr::NonNull,
    sync::Arc,
};

use migo_capi_abi::surface::{
    MIGO_PLATFORM_WAYLAND_SURFACE, MIGO_PLATFORM_X11_WINDOW, SurfaceDescriptorRef,
    ValidatedPlatformSurface,
};
use shared::surface::SurfaceRef;

use migo_capi_abi::{MIGO_ERROR_INTERNAL, MIGO_ERROR_UNSUPPORTED_PLATFORM, MigoResult};

/// Native identity retained for a later resize.
///
/// These are copied tokens, never pointers into caller-owned descriptor
/// storage. The host owns the X11/Wayland objects and their event loops.
#[derive(Clone, Copy)]
pub(crate) enum PlatformTarget {
    X11 {
        display: NonNull<c_void>,
        window: c_ulong,
    },
    Wayland {
        surface: NonNull<c_void>,
        display: NonNull<c_void>,
    },
}

// SAFETY: these are copied native identity tokens, never Rust references and
// never dereferenced by this type. Native/EGL access happens only through the
// platform Surface wrappers on the render lifecycle defined by the host.
unsafe impl Send for PlatformTarget {}
unsafe impl Sync for PlatformTarget {}

pub(crate) const fn supported_platform_kinds() -> u64 {
    (1u64 << MIGO_PLATFORM_X11_WINDOW) | (1u64 << MIGO_PLATFORM_WAYLAND_SURFACE)
}

pub(crate) fn rebuild_surface(
    target: PlatformTarget,
    width: u32,
    height: u32,
) -> Result<SurfaceRef, MigoResult> {
    match target {
        PlatformTarget::X11 { display, window } => Ok(Arc::new(
            platform::linux::presenter::LinuxX11Surface::new(display, window, width, height),
        )),
        PlatformTarget::Wayland { surface, display } => Ok(Arc::new(
            platform::linux::presenter::LinuxWaylandSurface::new(display, surface, width, height),
        )),
    }
}

/// Turn a fully copied and validated ABI value into Linux engine objects.
pub(crate) fn build_target(
    descriptor: SurfaceDescriptorRef,
) -> Result<
    (
        SurfaceRef,
        graphics::egl_platform::GraphicsPlatform,
        PlatformTarget,
    ),
    MigoResult,
> {
    let configuration = descriptor.configuration();
    match descriptor.platform() {
        ValidatedPlatformSurface::X11 {
            display, window, ..
        } => {
            let window = window as c_ulong;
            let surface: SurfaceRef = Arc::new(platform::linux::presenter::LinuxX11Surface::new(
                display,
                window,
                configuration.width_pixels(),
                configuration.height_pixels(),
            ));
            let graphics_platform = platform::linux::presenter::linux_x11_graphics_platform(
                display,
            )
            .map_err(|error| {
                tracing::error!("build_target: X11 graphics platform: {error:?}");
                MIGO_ERROR_INTERNAL
            })?;
            Ok((
                surface,
                graphics_platform,
                PlatformTarget::X11 { display, window },
            ))
        }
        ValidatedPlatformSurface::Wayland { display, surface } => {
            let surface_ref: SurfaceRef =
                Arc::new(platform::linux::presenter::LinuxWaylandSurface::new(
                    display,
                    surface,
                    configuration.width_pixels(),
                    configuration.height_pixels(),
                ));
            let graphics_platform = platform::linux::presenter::linux_wayland_graphics_platform(
                display,
            )
            .map_err(|error| {
                tracing::error!("build_target: Wayland graphics platform: {error:?}");
                MIGO_ERROR_INTERNAL
            })?;
            Ok((
                surface_ref,
                graphics_platform,
                PlatformTarget::Wayland { surface, display },
            ))
        }
        ValidatedPlatformSurface::Android { .. } => Err(MIGO_ERROR_UNSUPPORTED_PLATFORM),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_capability_mask_has_exactly_x11_and_wayland() {
        assert_eq!(
            supported_platform_kinds(),
            (1u64 << MIGO_PLATFORM_X11_WINDOW) | (1u64 << MIGO_PLATFORM_WAYLAND_SURFACE)
        );
    }
}
