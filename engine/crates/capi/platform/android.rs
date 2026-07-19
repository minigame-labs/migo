//! `ANativeWindow` surface construction for Android hosts.

use std::{ffi::c_void, ptr::NonNull, sync::Arc};

use shared::surface::SurfaceRef;

use crate::{
    abi::{
        validate_header, MigoResult, VersionedHeader, MIGO_ERROR_INTERNAL,
        MIGO_ERROR_INVALID_ARGUMENT, MIGO_ERROR_UNSUPPORTED_PLATFORM,
    },
    surface::{MigoAndroidNativeWindowDescriptor, MigoSurfaceDescriptor},
    MIGO_PLATFORM_ANDROID_NATIVE_WINDOW,
};

/// What a resize needs in order to rebuild the surface.
///
/// The pointer is the host's window, which outlives the attachment: the host
/// owns it, and the engine holds a strong reference of its own for as long as a
/// surface wraps it. A resize builds a new wrapper around the same window.
#[derive(Clone, Copy)]
pub(crate) enum PlatformTarget {
    NativeWindow { window: *mut c_void },
}

// The pointer is only ever handed back to ANativeWindow APIs, which are
// thread-safe, and the engine holds its own reference for the lifetime of each
// surface. Being able to send it is what lets a resize arrive on a thread other
// than the one that attached.
unsafe impl Send for PlatformTarget {}
unsafe impl Sync for PlatformTarget {}

pub(crate) fn rebuild_surface(target: PlatformTarget, width: u32, height: u32) -> SurfaceRef {
    match target {
        PlatformTarget::NativeWindow { window } => {
            // Acquire again for the new wrapper rather than sharing: each
            // wrapper releases in Drop, so two wrappers must not hold one
            // reference between them.
            let wrapper = unsafe {
                platform::android::surface::AndroidSurfaceWrapper::from_borrowed_acquire(
                    window.cast(),
                    width,
                    height,
                )
            }
            .expect("window pointer was validated at attach");
            Arc::new(wrapper)
        }
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
    if descriptor.platform_kind != MIGO_PLATFORM_ANDROID_NATIVE_WINDOW {
        return Err(MIGO_ERROR_UNSUPPORTED_PLATFORM);
    }
    // The envelope's size field and the payload's own struct_size are an
    // intentional cross-check; disagreeing means the caller mismatched them.
    if descriptor.platform_descriptor_size as usize
        != size_of::<MigoAndroidNativeWindowDescriptor>()
    {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    unsafe {
        validate_header(
            descriptor.platform_descriptor as *const VersionedHeader,
            size_of::<MigoAndroidNativeWindowDescriptor>(),
        )
    }?;
    let android =
        unsafe { &*(descriptor.platform_descriptor as *const MigoAndroidNativeWindowDescriptor) };
    if android.platform_kind != MIGO_PLATFORM_ANDROID_NATIVE_WINDOW {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    let Some(window) = NonNull::new(android.native_window) else {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    };
    if descriptor.width_pixels == 0 || descriptor.height_pixels == 0 {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }

    // from_borrowed_acquire, not from_surface_owned: the host keeps its own
    // reference to the window, so the engine takes one of its own and releases
    // it in Drop. This is the rule include/migo/platform/android.h states.
    let wrapper = unsafe {
        platform::android::surface::AndroidSurfaceWrapper::from_borrowed_acquire(
            window.as_ptr().cast(),
            descriptor.width_pixels,
            descriptor.height_pixels,
        )
    }
    .map_err(|error| {
        tracing::error!("build_target: native window: {error}");
        MIGO_ERROR_INVALID_ARGUMENT
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
            window: window.as_ptr(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::MIGO_ABI_VERSION_CURRENT;

    fn envelope(
        platform_kind: u32,
        descriptor_size: u32,
        payload: *const c_void,
    ) -> MigoSurfaceDescriptor {
        MigoSurfaceDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoSurfaceDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            generation: 1,
            platform_kind,
            flags: 0,
            width_pixels: 720,
            height_pixels: 1280,
            scale_factor: 3.0,
            color_space: 0,
            alpha_mode: 0,
            preferred_presentation_mode: 0,
            capability_flags: 0,
            platform_descriptor_size: descriptor_size,
            reserved0: 0,
            platform_descriptor: payload,
        }
    }

    fn payload(window: *mut c_void) -> MigoAndroidNativeWindowDescriptor {
        MigoAndroidNativeWindowDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoAndroidNativeWindowDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            platform_kind: MIGO_PLATFORM_ANDROID_NATIVE_WINDOW,
            flags: 0,
            native_window: window,
        }
    }

    #[test]
    fn other_platforms_are_reported_as_unsupported_not_invalid() {
        // A host on a platform this build does not implement should learn that,
        // rather than think its descriptor was malformed.
        let descriptor = envelope(
            6, // MIGO_PLATFORM_X11_WINDOW
            size_of::<MigoAndroidNativeWindowDescriptor>() as u32,
            std::ptr::null(),
        );
        assert_eq!(
            unsafe { build_target(&descriptor) }.err(),
            Some(MIGO_ERROR_UNSUPPORTED_PLATFORM)
        );
    }

    #[test]
    fn descriptor_size_mismatch_is_rejected() {
        let android = payload(0xdead_beef_usize as *mut c_void);
        let descriptor = envelope(
            MIGO_PLATFORM_ANDROID_NATIVE_WINDOW,
            8, // wrong on purpose
            &android as *const _ as *const c_void,
        );
        assert_eq!(
            unsafe { build_target(&descriptor) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn a_null_window_is_rejected_before_any_native_call() {
        // Rejecting here is what keeps a null from reaching
        // ANativeWindow_acquire, where it would be undefined behaviour rather
        // than an error code.
        let android = payload(std::ptr::null_mut());
        let descriptor = envelope(
            MIGO_PLATFORM_ANDROID_NATIVE_WINDOW,
            size_of::<MigoAndroidNativeWindowDescriptor>() as u32,
            &android as *const _ as *const c_void,
        );
        assert_eq!(
            unsafe { build_target(&descriptor) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn a_zero_sized_surface_is_rejected() {
        let android = payload(0xdead_beef_usize as *mut c_void);
        let mut descriptor = envelope(
            MIGO_PLATFORM_ANDROID_NATIVE_WINDOW,
            size_of::<MigoAndroidNativeWindowDescriptor>() as u32,
            &android as *const _ as *const c_void,
        );
        descriptor.width_pixels = 0;
        assert_eq!(
            unsafe { build_target(&descriptor) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }
}
