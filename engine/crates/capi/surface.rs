//! Surface attach, update and detach.
//!
//! Split out of `lib.rs` because these three entry points, the descriptors they
//! read, and the attachment handle they hand back are one concern: they are the
//! only place the ABI touches a host's window.
//!
//! The engine is told about a resize the same way Android tells it -- a fresh
//! generation-tagged lease delivered as `HostCommand::UpdateSurface` (see
//! `platform/android/jni/inbound.rs`). Using the one path keeps a resize from
//! meaning different things on different platforms.

use std::{
    ffi::{c_ulong, c_void},
    ptr::NonNull,
    sync::Arc,
};

use core::{lease_surface, retire_surface, send_critical_command_to_host, spawn_host_thread,
           PlatformServices};
use shared::{config::InitOptions, protocol::host_cmd::HostCommand, surface::SurfaceRef};

use crate::{
    abi::{
        guard, validate_header, MigoResult, VersionedHeader, MIGO_ERROR_INTERNAL,
        MIGO_ERROR_INVALID_ARGUMENT, MIGO_ERROR_INVALID_STATE, MIGO_ERROR_UNSUPPORTED_PLATFORM,
        MIGO_OK,
    },
    callbacks, host_kit, MigoSession, MIGO_PLATFORM_X11_WINDOW,
};

// ---- C struct mirrors -------------------------------------------------------

#[repr(C)]
pub struct MigoSurfaceDescriptor {
    pub(crate) header: VersionedHeader,
    pub(crate) generation: u64,
    pub(crate) platform_kind: u32,
    pub(crate) flags: u32,
    pub(crate) width_pixels: u32,
    pub(crate) height_pixels: u32,
    pub(crate) scale_factor: f32,
    pub(crate) color_space: u32,
    pub(crate) alpha_mode: u32,
    pub(crate) preferred_presentation_mode: u32,
    pub(crate) capability_flags: u64,
    pub(crate) platform_descriptor_size: u32,
    pub(crate) reserved0: u32,
    pub(crate) platform_descriptor: *const c_void,
}

/// `MigoSurfaceMetrics` from `include/migo/surface.h`. A resize supplies only
/// the values that can change; the host is not asked to re-describe its window.
#[repr(C)]
pub struct MigoSurfaceMetrics {
    pub(crate) header: VersionedHeader,
    pub(crate) generation: u64,
    pub(crate) width_pixels: u32,
    pub(crate) height_pixels: u32,
    pub(crate) scale_factor: f32,
    pub(crate) color_space: u32,
    pub(crate) alpha_mode: u32,
    pub(crate) preferred_presentation_mode: u32,
    pub(crate) flags: u32,
    pub(crate) reserved0: u32,
}

#[repr(C)]
pub struct MigoX11WindowDescriptor {
    pub(crate) header: VersionedHeader,
    pub(crate) platform_kind: u32,
    pub(crate) flags: u32,
    pub(crate) display: *mut c_void,
    pub(crate) window: usize,
    pub(crate) screen: i32,
    pub(crate) reserved0: u32,
}

// ---- Attachment handle ------------------------------------------------------

/// What an update needs in order to rebuild the surface.
///
/// The values are copied rather than the descriptor pointer retained: the
/// header says `platform_descriptor` is borrowed for the attach call only, so
/// keeping it would outlive the caller's storage.
#[derive(Clone, Copy)]
enum PlatformTarget {
    X11 { window: c_ulong },
}

pub struct MigoSurfaceAttachment {
    session: NonNull<MigoSession>,
    generation: u64,
    target: PlatformTarget,
    width_pixels: u32,
    height_pixels: u32,
}

impl MigoSurfaceAttachment {
    fn rebuild_surface(&self, width: u32, height: u32) -> SurfaceRef {
        match self.target {
            PlatformTarget::X11 { window } => Arc::new(
                platform::desktop::presenter::LinuxX11Surface::new(window, width, height),
            ),
        }
    }
}

// ---- Entry points -----------------------------------------------------------

/// # Safety
/// `session` must be live; `descriptor` a `MigoSurfaceDescriptor` whose
/// `platform_descriptor` points at the matching typed descriptor;
/// `out_attachment` writable storage for one pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_session_attach_surface(
    session: *mut MigoSession,
    descriptor: *const MigoSurfaceDescriptor,
    out_attachment: *mut *mut MigoSurfaceAttachment,
) -> MigoResult {
    guard("migo_session_attach_surface", || {
        let Some(session_ptr) = NonNull::new(session) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        let Some(out_attachment) = (unsafe { out_attachment.as_mut() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        if let Err(error) = unsafe {
            validate_header(
                descriptor as *const VersionedHeader,
                size_of::<MigoSurfaceDescriptor>(),
            )
        } {
            return error;
        }
        let descriptor = unsafe { &*descriptor };

        let (surface, graphics_platform, target) = match unsafe { build_target(descriptor) } {
            Ok(built) => built,
            Err(error) => return error,
        };

        let session_ref = unsafe { session_ptr.as_ref() };
        let Ok(mut state) = session_ref.state.lock() else {
            return MIGO_ERROR_INTERNAL;
        };
        if state.attached {
            return MIGO_ERROR_INVALID_STATE;
        }

        match state.host {
            // Re-attach after a detach: the host thread is still alive, so the
            // surface joins it as a new generation rather than starting over.
            // The lease is delivered, not dropped -- dropping it would retire
            // the generation it just created and leave nothing presenting.
            Some(host) => match lease_surface(host, surface) {
                Ok(lease) => {
                    if let Err(error) =
                        send_critical_command_to_host(host, HostCommand::UpdateSurface { lease })
                    {
                        tracing::error!("migo_session_attach_surface: send failed: {error}");
                        return MIGO_ERROR_INTERNAL;
                    }
                }
                Err(error) => {
                    tracing::error!("migo_session_attach_surface: lease failed: {error}");
                    return MIGO_ERROR_INTERNAL;
                }
            },
            None => {
                let notifier = state.callbacks.map(|callbacks| {
                    callbacks::Notifier::new(
                        callbacks,
                        session_ptr.cast(),
                        Arc::clone(&session_ref.alive),
                    )
                });
                let host_kit: Arc<dyn PlatformServices> =
                    Arc::new(host_kit::CapiHostKit::new(notifier));
                let options = InitOptions::new()
                    .with_files_dir(session_ref.engine.files_dir.clone())
                    .with_cache_dir(session_ref.engine.cache_dir.clone())
                    .with_code_cache_dir(session_ref.engine.code_cache_dir.clone())
                    .with_pixel_ratio(descriptor.scale_factor.max(1.0))
                    .with_target_fps(60)
                    .with_code_signing_enabled(!session_ref.engine.allow_unsigned_content);
                match spawn_host_thread(surface, graphics_platform, host_kit, options) {
                    Ok(host) => state.host = Some(host),
                    Err(error) => {
                        tracing::error!("migo_session_attach_surface: spawn failed: {error:?}");
                        return MIGO_ERROR_INTERNAL;
                    }
                }
            }
        }
        state.attached = true;

        let attachment = Box::new(MigoSurfaceAttachment {
            session: session_ptr,
            generation: descriptor.generation,
            target,
            width_pixels: descriptor.width_pixels,
            height_pixels: descriptor.height_pixels,
        });
        *out_attachment = Box::into_raw(attachment);
        MIGO_OK
    })
}

/// Report a resize or a presentation-parameter change.
///
/// # Safety
/// `attachment` must come from [`migo_session_attach_surface`] and still be
/// live; `metrics` must point at a `MigoSurfaceMetrics` whose `struct_size`
/// matches the caller's ABI version.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_surface_update(
    attachment: *mut MigoSurfaceAttachment,
    metrics: *const MigoSurfaceMetrics,
) -> MigoResult {
    guard("migo_surface_update", || {
        let Some(attachment) = (unsafe { attachment.as_mut() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        if let Err(error) = unsafe {
            validate_header(
                metrics as *const VersionedHeader,
                size_of::<MigoSurfaceMetrics>(),
            )
        } {
            return error;
        }
        let metrics = unsafe { &*metrics };
        if metrics.width_pixels == 0 || metrics.height_pixels == 0 {
            return MIGO_ERROR_INVALID_ARGUMENT;
        }

        let session = unsafe { attachment.session.as_ref() };
        let Ok(state) = session.state.lock() else {
            return MIGO_ERROR_INTERNAL;
        };
        // Reporting success with nothing attached would tell the host its
        // resize took effect when no surface exists to resize.
        if !state.attached {
            return MIGO_ERROR_INVALID_STATE;
        }
        let Some(host) = state.host else {
            return MIGO_ERROR_INVALID_STATE;
        };

        let surface = attachment.rebuild_surface(metrics.width_pixels, metrics.height_pixels);
        let lease = match lease_surface(host, surface) {
            Ok(lease) => lease,
            Err(error) => {
                tracing::error!("migo_surface_update: lease failed: {error}");
                return MIGO_ERROR_INTERNAL;
            }
        };
        // Critical, like Android's path: a dropped resize strands the app on a
        // stale-sized frame with no further callback to recover from.
        if let Err(error) = send_critical_command_to_host(host, HostCommand::UpdateSurface { lease })
        {
            tracing::error!("migo_surface_update: send failed: {error}");
            return MIGO_ERROR_INTERNAL;
        }

        attachment.width_pixels = metrics.width_pixels;
        attachment.height_pixels = metrics.height_pixels;
        MIGO_OK
    })
}

/// # Safety
/// `attachment` must come from [`migo_session_attach_surface`] and its session
/// must still be live. On `MIGO_OK` the handle is consumed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_surface_detach(
    attachment: *mut MigoSurfaceAttachment,
) -> MigoResult {
    guard("migo_surface_detach", || {
        if attachment.is_null() {
            return MIGO_ERROR_INVALID_ARGUMENT;
        }
        let attachment = unsafe { Box::from_raw(attachment) };
        let session = unsafe { attachment.session.as_ref() };
        let Ok(mut state) = session.state.lock() else {
            return MIGO_ERROR_INTERNAL;
        };
        if let Some(host) = state.host {
            // Retiring the generation is the synchronous completion boundary the
            // header promises: no later present may reference it.
            if let Err(error) = retire_surface(host) {
                tracing::warn!("migo_surface_detach: retire failed: {error}");
            }
        }
        state.attached = false;
        MIGO_OK
    })
}

/// Translate a validated descriptor into the engine's surface, graphics
/// platform, and the values an update needs later.
///
/// # Safety
/// `descriptor` must have passed [`validate_header`].
unsafe fn build_target(
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
            tracing::error!("migo_session_attach_surface: graphics platform: {error:?}");
            MIGO_ERROR_INTERNAL
        })?;
    Ok((surface, graphics_platform, PlatformTarget::X11 { window }))
}

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_UNSUPPORTED_ABI};
    use crate::test_support::with_session;

    fn metrics(width: u32, height: u32) -> MigoSurfaceMetrics {
        MigoSurfaceMetrics {
            header: VersionedHeader {
                struct_size: size_of::<MigoSurfaceMetrics>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            generation: 1,
            width_pixels: width,
            height_pixels: height,
            scale_factor: 1.0,
            color_space: 0,
            alpha_mode: 0,
            preferred_presentation_mode: 0,
            flags: 0,
            reserved0: 0,
        }
    }

    /// An attachment that is structurally valid but whose session has no host.
    ///
    /// Built directly rather than through attach so these tests need no X11
    /// display: every path they exercise rejects the call before it would reach
    /// the engine. The session behind it is real, so the state lock and the
    /// `attached` check are the production ones.
    fn detached_attachment(session: *mut MigoSession) -> MigoSurfaceAttachment {
        MigoSurfaceAttachment {
            session: NonNull::new(session).expect("session"),
            generation: 1,
            target: PlatformTarget::X11 { window: 0x2a0_0001 },
            width_pixels: 640,
            height_pixels: 480,
        }
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
        let describe = |x11: &MigoX11WindowDescriptor| MigoSurfaceDescriptor {
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
            unsafe { build_target(&describe(&no_display)) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );

        let no_window = make(0xdead_beef_usize as *mut c_void, 0);
        assert_eq!(
            unsafe { build_target(&describe(&no_window)) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn update_rejects_a_null_attachment() {
        let m = metrics(800, 600);
        assert_eq!(
            unsafe { migo_surface_update(std::ptr::null_mut(), &m) },
            MIGO_ERROR_INVALID_ARGUMENT
        );
    }

    #[test]
    fn update_rejects_null_metrics() {
        with_session("update-null-metrics", |session| {
            let mut attachment = detached_attachment(session);
            assert_eq!(
                unsafe { migo_surface_update(&mut attachment, std::ptr::null()) },
                MIGO_ERROR_INVALID_ARGUMENT
            );
        });
    }

    #[test]
    fn update_rejects_a_struct_size_mismatch() {
        with_session("update-size-mismatch", |session| {
            let mut attachment = detached_attachment(session);
            let mut m = metrics(800, 600);
            m.header.struct_size = 8;
            assert_eq!(
                unsafe { migo_surface_update(&mut attachment, &m) },
                MIGO_ERROR_INVALID_ARGUMENT
            );
        });
    }

    #[test]
    fn update_rejects_an_unknown_abi_version() {
        with_session("update-abi-version", |session| {
            let mut attachment = detached_attachment(session);
            let mut m = metrics(800, 600);
            m.header.abi_version = 99;
            assert_eq!(
                unsafe { migo_surface_update(&mut attachment, &m) },
                MIGO_ERROR_UNSUPPORTED_ABI
            );
        });
    }

    #[test]
    fn update_rejects_a_zero_sized_surface() {
        with_session("update-zero-size", |session| {
            let mut attachment = detached_attachment(session);
            assert_eq!(
                unsafe { migo_surface_update(&mut attachment, &metrics(0, 600)) },
                MIGO_ERROR_INVALID_ARGUMENT
            );
            assert_eq!(
                unsafe { migo_surface_update(&mut attachment, &metrics(800, 0)) },
                MIGO_ERROR_INVALID_ARGUMENT
            );
        });
    }

    #[test]
    fn update_on_a_detached_session_reports_invalid_state() {
        // Nothing is attached, so there is no surface to resize. Reporting
        // success would tell the host its resize took effect when it did not.
        with_session("update-detached", |session| {
            let mut attachment = detached_attachment(session);
            assert_eq!(
                unsafe { migo_surface_update(&mut attachment, &metrics(800, 600)) },
                MIGO_ERROR_INVALID_STATE
            );
        });
    }

    // A null-attachment detach is covered by
    // `tests::null_handles_are_argument_errors_not_crashes`, which checks every
    // handle-taking entry point in one place.
}
