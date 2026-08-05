//! `OHNativeWindow` surface construction for OpenHarmony hosts.

use std::{ffi::c_void, sync::Arc};

use graphics::egl_platform::GraphicsPlatform;
use migo_capi_abi::surface::{
    MIGO_PLATFORM_OPENHARMONY_NATIVE_WINDOW, SurfaceDescriptorRef, ValidatedPlatformSurface,
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
/// The host contract keeps the window alive through attachment retirement.
/// Every engine surface wrapper takes and drops its own native-object
/// reference, so overlapping old/new GPU generations never share one.
#[derive(Clone, Copy)]
pub(crate) enum PlatformTarget {
    NativeWindow {
        window: *mut c_void,
    },
    #[cfg(test)]
    TestOnly,
}

// OpenHarmony's native window reference counting and buffer APIs are internally
// synchronized. The token is never dereferenced as Rust memory; it is only
// passed back to platform APIs.
unsafe impl Send for PlatformTarget {}
unsafe impl Sync for PlatformTarget {}

pub(crate) const fn supported_platform_kinds() -> u64 {
    1u64 << MIGO_PLATFORM_OPENHARMONY_NATIVE_WINDOW
}

pub(crate) fn rebuild_surface(
    target: PlatformTarget,
    width: u32,
    height: u32,
) -> Result<SurfaceRef, MigoResult> {
    match target {
        PlatformTarget::NativeWindow { window } => {
            // A separate reference is required because both the retiring and
            // the new render generation may coexist until the render thread
            // fences the old one.
            let wrapper = unsafe {
                platform::ohos::surface::OhosSurfaceWrapper::from_borrowed_reference(
                    window.cast(),
                    width,
                    height,
                )
            }
            .map_err(|error| {
                tracing::error!("rebuild_surface: native window: {error}");
                MIGO_ERROR_INTERNAL
            })?;
            Ok(Arc::new(wrapper))
        }
        #[cfg(test)]
        PlatformTarget::TestOnly => Err(MIGO_ERROR_UNSUPPORTED_PLATFORM),
    }
}

/// Turn a fully copied and validated ABI value into OpenHarmony engine objects.
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
    let ValidatedPlatformSurface::OpenHarmony { native_window } = descriptor.platform() else {
        return Err(MIGO_ERROR_UNSUPPORTED_PLATFORM);
    };
    let configuration = descriptor.configuration();

    // The host retains its own reference; Migo takes an independent one and the
    // wrapper drops exactly that one from Drop.
    let wrapper = unsafe {
        platform::ohos::surface::OhosSurfaceWrapper::from_borrowed_reference(
            native_window.as_ptr().cast(),
            configuration.width_pixels(),
            configuration.height_pixels(),
        )
    }
    .map_err(|error| {
        tracing::error!("build_target: native window: {error}");
        migo_capi_abi::MIGO_ERROR_INVALID_ARGUMENT
    })?;

    let graphics_platform = match existing {
        Some(PlatformContext::Graphics(graphics_platform)) => graphics_platform.clone(),
        #[cfg(test)]
        Some(PlatformContext::TestOnly) => return Err(MIGO_ERROR_INTERNAL),
        None => platform::ohos::presenter::ohos_graphics_platform().map_err(|error| {
            tracing::error!("build_target: graphics platform: {error:?}");
            MIGO_ERROR_INTERNAL
        })?,
    };

    Ok((
        Arc::new(wrapper),
        graphics_platform.clone(),
        PlatformTarget::NativeWindow {
            window: native_window.as_ptr(),
        },
        PlatformContext::Graphics(graphics_platform),
    ))
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
    fn ohos_capability_mask_has_exactly_the_native_window_kind() {
        assert_eq!(
            supported_platform_kinds(),
            1u64 << MIGO_PLATFORM_OPENHARMONY_NATIVE_WINDOW
        );
    }
}
