//! Win32 surface construction for Windows hosts.

use std::{ffi::c_void, ptr::NonNull, sync::Arc};

use graphics::egl_platform::GraphicsPlatform;
use migo_capi_abi::surface::{
    MIGO_PLATFORM_WIN32_HWND, SurfaceDescriptorRef, ValidatedPlatformSurface,
};
use shared::surface::SurfaceRef;

use migo_capi_abi::{MIGO_ERROR_INTERNAL, MIGO_ERROR_UNSUPPORTED_PLATFORM, MigoResult};

#[derive(Clone, Debug)]
pub(crate) enum PlatformContext {
    Graphics(GraphicsPlatform),
    #[cfg(test)]
    TestOnly,
}

/// Native identity retained for a later resize.
///
/// A copied token, never a pointer into caller-owned descriptor storage. The
/// host owns the window and its message loop; Migo neither destroys the window
/// nor pumps messages for it.
#[derive(Clone, Copy)]
pub(crate) enum PlatformTarget {
    Win32 {
        hwnd: NonNull<c_void>,
    },
    #[cfg(test)]
    TestOnly,
}

// SAFETY: this is a copied native identity token, never a Rust reference and
// never dereferenced by this type. Native/EGL access happens only through the
// platform Surface wrapper on the render lifecycle defined by the host.
unsafe impl Send for PlatformTarget {}
unsafe impl Sync for PlatformTarget {}

pub(crate) const fn supported_platform_kinds() -> u64 {
    1u64 << MIGO_PLATFORM_WIN32_HWND
}

pub(crate) fn rebuild_surface(
    target: PlatformTarget,
    width: u32,
    height: u32,
) -> Result<SurfaceRef, MigoResult> {
    match target {
        // SAFETY: the handle reached us through a validated descriptor, and the
        // header obliges the host to keep it live until the release observer
        // reports RELEASED. A rebuild happens strictly inside that window.
        PlatformTarget::Win32 { hwnd } => Ok(Arc::new(unsafe {
            platform::windows::presenter::WindowsHwndSurface::new(hwnd, width, height)
        })),
        #[cfg(test)]
        PlatformTarget::TestOnly => Err(MIGO_ERROR_UNSUPPORTED_PLATFORM),
    }
}

/// Turn a fully copied and validated ABI value into Windows engine objects.
pub(crate) fn build_target(
    descriptor: SurfaceDescriptorRef,
    existing: Option<&PlatformContext>,
) -> Result<
    (
        SurfaceRef,
        GraphicsPlatform,
        PlatformTarget,
        PlatformContext,
    ),
    MigoResult,
> {
    let configuration = descriptor.configuration();
    match descriptor.platform() {
        ValidatedPlatformSurface::Win32 { hwnd } => {
            // SAFETY: as in `rebuild_surface` -- validated non-null by the ABI
            // parse, and kept live by the host until release completes.
            let surface: SurfaceRef = Arc::new(unsafe {
                platform::windows::presenter::WindowsHwndSurface::new(
                    hwnd,
                    configuration.width_pixels(),
                    configuration.height_pixels(),
                )
            });
            let graphics_platform = match existing {
                Some(PlatformContext::Graphics(graphics_platform)) => graphics_platform.clone(),
                #[cfg(test)]
                Some(PlatformContext::TestOnly) => return Err(MIGO_ERROR_INTERNAL),
                None => platform::windows::presenter::windows_hwnd_graphics_platform().map_err(
                    |error| {
                        tracing::error!("build_target: Win32 graphics platform: {error:?}");
                        MIGO_ERROR_INTERNAL
                    },
                )?,
            };
            Ok((
                surface,
                graphics_platform.clone(),
                PlatformTarget::Win32 { hwnd },
                PlatformContext::Graphics(graphics_platform),
            ))
        }
        // Listed rather than matched with a wildcard: a new platform payload
        // must not compile until every platform module has taken a position on
        // it. A build that silently accepts nothing is the defect this module
        // was added to fix.
        ValidatedPlatformSurface::Android { .. }
        | ValidatedPlatformSurface::X11 { .. }
        | ValidatedPlatformSurface::Wayland { .. }
        | ValidatedPlatformSurface::OpenHarmony { .. } => Err(MIGO_ERROR_UNSUPPORTED_PLATFORM),
    }
}

#[cfg(test)]
pub(crate) fn test_platform_target() -> PlatformTarget {
    PlatformTarget::TestOnly
}

#[cfg(test)]
pub(crate) fn test_platform_context() -> PlatformContext {
    PlatformContext::TestOnly
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_capability_mask_has_exactly_win32_hwnd() {
        assert_eq!(supported_platform_kinds(), 1u64 << MIGO_PLATFORM_WIN32_HWND);
    }

    #[test]
    fn capability_mask_is_not_empty() {
        // The published windows-sdk-0.1.0 exported every entry point, loaded
        // cleanly, and reported a mask of zero, so it could attach nothing. A
        // mask of zero is what "no platform layer" looks like from outside.
        assert_ne!(supported_platform_kinds(), 0);
    }
}
