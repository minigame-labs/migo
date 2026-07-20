//! `ANativeWindow` surface construction for Android hosts.

use std::{ffi::c_void, sync::Arc};

use migo_capi_abi::surface::{
    MIGO_PLATFORM_ANDROID_NATIVE_WINDOW, SurfaceDescriptorRef, ValidatedPlatformSurface,
};
use shared::surface::SurfaceRef;

use crate::abi::{MIGO_ERROR_INTERNAL, MIGO_ERROR_UNSUPPORTED_PLATFORM, MigoResult};

/// Native identity retained for a later resize.
///
/// The host contract keeps the window alive through attachment retirement.
/// Every engine surface wrapper acquires and releases its own strong native
/// reference, so overlapping old/new GPU generations never share one ref.
#[derive(Clone, Copy)]
pub(crate) enum PlatformTarget {
    NativeWindow { window: *mut c_void },
}

// ANativeWindow reference counting and buffer APIs are thread-safe. The token
// is never dereferenced as Rust memory; it is only passed back to NDK APIs.
unsafe impl Send for PlatformTarget {}
unsafe impl Sync for PlatformTarget {}

pub(crate) const fn supported_platform_kinds() -> u64 {
    1u64 << MIGO_PLATFORM_ANDROID_NATIVE_WINDOW
}

pub(crate) fn rebuild_surface(
    target: PlatformTarget,
    width: u32,
    height: u32,
) -> Result<SurfaceRef, MigoResult> {
    match target {
        PlatformTarget::NativeWindow { window } => {
            // A separate acquire is required because both the retiring and new
            // render generations may coexist until the render thread fences
            // the old one.
            let wrapper = unsafe {
                platform::android::surface::AndroidSurfaceWrapper::from_borrowed_acquire(
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
    }
}

/// Turn a fully copied and validated ABI value into Android engine objects.
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
    let ValidatedPlatformSurface::Android { native_window } = descriptor.platform() else {
        return Err(MIGO_ERROR_UNSUPPORTED_PLATFORM);
    };
    let configuration = descriptor.configuration();

    // The host retains its reference; Migo acquires an independent one and the
    // wrapper releases exactly that reference from Drop.
    let wrapper = unsafe {
        platform::android::surface::AndroidSurfaceWrapper::from_borrowed_acquire(
            native_window.as_ptr().cast(),
            configuration.width_pixels(),
            configuration.height_pixels(),
        )
    }
    .map_err(|error| {
        tracing::error!("build_target: native window: {error}");
        migo_capi_abi::MIGO_ERROR_INVALID_ARGUMENT
    })?;

    let graphics_platform =
        platform::android::presenter::android_graphics_platform().map_err(|error| {
            tracing::error!("build_target: graphics platform: {error:?}");
            MIGO_ERROR_INTERNAL
        })?;

    Ok((
        Arc::new(wrapper),
        graphics_platform,
        PlatformTarget::NativeWindow {
            window: native_window.as_ptr(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_capability_mask_has_exactly_the_native_window_kind() {
        assert_eq!(
            supported_platform_kinds(),
            1u64 << MIGO_PLATFORM_ANDROID_NATIVE_WINDOW
        );
    }
}
