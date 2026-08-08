//! X11 and Wayland surface construction for Linux hosts.

use std::{
    ffi::{c_ulong, c_void},
    ptr::NonNull,
    sync::Arc,
};

use graphics::egl_platform::GraphicsPlatform;
use migo_capi_abi::surface::{
    MIGO_PLATFORM_WAYLAND_SURFACE, MIGO_PLATFORM_X11_WINDOW, SurfaceDescriptorRef,
    ValidatedPlatformSurface,
};
use platform::linux::presenter::{
    LinuxWaylandSurface, LinuxX11Context, linux_wayland_graphics_platform,
};
use shared::surface::SurfaceRef;

use migo_capi_abi::{
    MIGO_ERROR_INTERNAL, MIGO_ERROR_INVALID_STATE, MIGO_ERROR_UNSUPPORTED_PLATFORM, MigoResult,
};

/// Immutable platform construction state retained for every Host lifetime.
#[derive(Clone, Debug)]
pub(crate) enum PlatformContext {
    X11(LinuxX11Context),
    Wayland {
        display: NonNull<c_void>,
        graphics_platform: GraphicsPlatform,
    },
    #[cfg(test)]
    TestOnly,
}

// SAFETY: X11 owns its thread-compatible private connection. The Wayland
// display is an opaque host-owned token whose lifetime contract spans every
// attachment; EGL access remains on Migo's graphics threads.
unsafe impl Send for PlatformContext {}
unsafe impl Sync for PlatformContext {}

/// Native identity retained for a later resize.
///
/// These are copied tokens, never pointers into caller-owned descriptor
/// storage. The host owns the X11/Wayland objects and their event loops.
#[derive(Clone)]
pub(crate) enum PlatformTarget {
    X11 {
        context: LinuxX11Context,
        window: c_ulong,
    },
    Wayland {
        surface: NonNull<c_void>,
        display: NonNull<c_void>,
    },
    #[cfg(test)]
    TestOnly,
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
        PlatformTarget::X11 { context, window } => Ok(context.surface(window, width, height)),
        PlatformTarget::Wayland { surface, display } => Ok(Arc::new(LinuxWaylandSurface::new(
            display, surface, width, height,
        ))),
        #[cfg(test)]
        PlatformTarget::TestOnly => Err(MIGO_ERROR_UNSUPPORTED_PLATFORM),
    }
}

/// Turn a fully copied and validated ABI value into Linux engine objects.
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
        ValidatedPlatformSurface::X11 {
            display, window, ..
        } => {
            let window = window as c_ulong;
            let context = match existing {
                Some(PlatformContext::X11(context)) => {
                    if let Err(error) = unsafe { context.supports_host_display(display) } {
                        tracing::error!("build_target: X11 context cannot be reused: {error:?}");
                        return Err(MIGO_ERROR_INVALID_STATE);
                    }
                    context.clone()
                }
                Some(_) => return Err(MIGO_ERROR_INVALID_STATE),
                None => unsafe { LinuxX11Context::open(display) }.map_err(|error| {
                    tracing::error!("build_target: open owned X11 context: {error:?}");
                    MIGO_ERROR_INTERNAL
                })?,
            };
            let graphics_platform = context.graphics_platform();
            let surface = context.surface(
                window,
                configuration.width_pixels(),
                configuration.height_pixels(),
            );
            Ok((
                surface,
                graphics_platform,
                PlatformTarget::X11 {
                    context: context.clone(),
                    window,
                },
                PlatformContext::X11(context),
            ))
        }
        ValidatedPlatformSurface::Wayland { display, surface } => {
            let graphics_platform = match existing {
                Some(PlatformContext::Wayland {
                    display: installed,
                    graphics_platform,
                }) if *installed == display => graphics_platform.clone(),
                Some(_) => return Err(MIGO_ERROR_INVALID_STATE),
                None => linux_wayland_graphics_platform(display).map_err(|error| {
                    tracing::error!("build_target: Wayland graphics platform: {error:?}");
                    MIGO_ERROR_INTERNAL
                })?,
            };
            let surface_ref: SurfaceRef = Arc::new(LinuxWaylandSurface::new(
                display,
                surface,
                configuration.width_pixels(),
                configuration.height_pixels(),
            ));
            Ok((
                surface_ref,
                graphics_platform.clone(),
                PlatformTarget::Wayland { surface, display },
                PlatformContext::Wayland {
                    display,
                    graphics_platform,
                },
            ))
        }
        // Listed rather than matched with a wildcard: a new platform payload
        // must not compile until every platform module has taken a position on
        // it.
        ValidatedPlatformSurface::Android { .. }
        | ValidatedPlatformSurface::Win32 { .. }
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
    use std::{ffi::c_void, mem::size_of};

    use migo_capi_abi::{
        MIGO_ABI_VERSION_CURRENT, VersionedHeader,
        surface::{MigoSurfaceDescriptor, MigoWaylandSurfaceDescriptor, MigoX11WindowDescriptor},
    };
    use platform::linux::presenter::X11TestServers;

    fn envelope(kind: u32, payload_size: usize, payload: *const c_void) -> MigoSurfaceDescriptor {
        MigoSurfaceDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoSurfaceDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            generation: 1,
            platform_kind: kind,
            flags: 0,
            width_pixels: 640,
            height_pixels: 480,
            scale_factor: 1.0,
            color_space: 0,
            alpha_mode: 0,
            preferred_presentation_mode: 0,
            capability_flags: 0,
            platform_descriptor_size: payload_size as u32,
            reserved0: 0,
            platform_descriptor: payload,
        }
    }

    fn wayland_descriptor(display: usize, surface: usize) -> SurfaceDescriptorRef {
        let payload = MigoWaylandSurfaceDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoWaylandSurfaceDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            platform_kind: MIGO_PLATFORM_WAYLAND_SURFACE,
            flags: 0,
            display: display as *mut c_void,
            surface: surface as *mut c_void,
        };
        let descriptor = envelope(
            MIGO_PLATFORM_WAYLAND_SURFACE,
            size_of::<MigoWaylandSurfaceDescriptor>(),
            (&payload as *const MigoWaylandSurfaceDescriptor).cast(),
        );
        unsafe { SurfaceDescriptorRef::parse(&descriptor) }.expect("valid Wayland descriptor")
    }

    fn x11_descriptor(display: usize) -> SurfaceDescriptorRef {
        let payload = MigoX11WindowDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoX11WindowDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            platform_kind: MIGO_PLATFORM_X11_WINDOW,
            flags: 0,
            display: display as *mut c_void,
            window: 0x2a0_0001,
            screen: 0,
            reserved0: 0,
        };
        let descriptor = envelope(
            MIGO_PLATFORM_X11_WINDOW,
            size_of::<MigoX11WindowDescriptor>(),
            (&payload as *const MigoX11WindowDescriptor).cast(),
        );
        unsafe { SurfaceDescriptorRef::parse(&descriptor) }.expect("valid X11 descriptor")
    }

    #[test]
    fn linux_capability_mask_has_exactly_x11_and_wayland() {
        assert_eq!(
            supported_platform_kinds(),
            (1u64 << MIGO_PLATFORM_X11_WINDOW) | (1u64 << MIGO_PLATFORM_WAYLAND_SURFACE)
        );
    }

    #[test]
    fn platform_context_reuses_the_exact_wayland_graphics_domain() {
        let (_, first_platform, _, context) =
            build_target(wayland_descriptor(0x1000, 0x2000), None).expect("cold Wayland target");
        let (_, reused_platform, _, returned_context) =
            build_target(wayland_descriptor(0x1000, 0x3000), Some(&context))
                .expect("same Wayland display must reuse its context");

        assert_eq!(
            first_platform.platform_identity(),
            reused_platform.platform_identity()
        );
        assert!(Arc::ptr_eq(
            first_platform.egl_provider(),
            reused_platform.egl_provider()
        ));
        assert!(Arc::ptr_eq(
            first_platform.surface_factory(),
            reused_platform.surface_factory()
        ));
        let PlatformContext::Wayland {
            display,
            graphics_platform,
        } = returned_context
        else {
            panic!("returned context changed platform kind");
        };
        assert_eq!(display.as_ptr() as usize, 0x1000);
        assert!(Arc::ptr_eq(
            first_platform.egl_provider(),
            graphics_platform.egl_provider()
        ));
    }

    #[test]
    fn platform_context_rejects_display_or_kind_change_before_native_access() {
        let (_, _, _, context) =
            build_target(wayland_descriptor(0x1000, 0x2000), None).expect("cold Wayland target");

        let different_display =
            build_target(wayland_descriptor(0x1001, 0x3000), Some(&context)).err();
        assert_eq!(different_display, Some(MIGO_ERROR_INVALID_STATE));

        // The X11 token is intentionally synthetic. A kind switch must fail
        // from the stored context before Xlib can inspect that pointer.
        let different_kind = build_target(x11_descriptor(0x4000), Some(&context)).err();
        assert_eq!(different_kind, Some(MIGO_ERROR_INVALID_STATE));
    }

    // ---- X11 ----
    //
    // These drive `build_target` down the X11 arm against a declared server
    // topology rather than a display server, so they run anywhere. What they
    // cannot cover is the cold arm: `existing: None` calls
    // `LinuxX11Context::open`, which loads Xlib and dereferences the host
    // `Display*`. That one needs a live server and is covered by
    // `x11_connection`'s `native_owned_connection_round_trip` under `xvfb-run`.

    const TEST_SERVER: u8 = 7;
    const OTHER_SERVER: u8 = 8;

    fn display(address: usize) -> NonNull<c_void> {
        NonNull::new(address as *mut c_void).expect("test display address is non-null")
    }

    /// A host display, its private render connection, and the stored context a
    /// second attach would find -- all on `TEST_SERVER`.
    fn attached_x11(servers: &mut X11TestServers) -> (NonNull<c_void>, PlatformContext) {
        let host = display(0x1000);
        let render = display(0x2000);
        servers.place(host, TEST_SERVER).place(render, TEST_SERVER);
        let context = LinuxX11Context::open_on_test_servers(servers, host, render)
            .expect("cold X11 attach must open the owned render connection");
        assert_eq!(
            servers.opens(),
            1,
            "a cold attach opens exactly one context"
        );
        (host, PlatformContext::X11(context))
    }

    #[test]
    fn x11_reattachment_to_the_same_server_reuses_the_one_owned_connection() {
        let mut servers = X11TestServers::new();
        let (host, stored) = attached_x11(&mut servers);
        let PlatformContext::X11(ref installed) = stored else {
            panic!("stored context changed platform kind");
        };
        let installed_platform = installed.graphics_platform();

        let (_, reused_platform, target, returned) =
            build_target(x11_descriptor(host.as_ptr() as usize), Some(&stored))
                .expect("the same X11 server must reattach");

        assert_eq!(
            servers.opens(),
            1,
            "reattachment must reuse the owned connection, not open a second one"
        );
        assert_eq!(
            installed_platform.platform_identity(),
            reused_platform.platform_identity()
        );
        assert!(Arc::ptr_eq(
            installed_platform.egl_provider(),
            reused_platform.egl_provider()
        ));
        assert!(Arc::ptr_eq(
            installed_platform.surface_factory(),
            reused_platform.surface_factory()
        ));
        let PlatformContext::X11(returned) = returned else {
            panic!("returned context changed platform kind");
        };
        assert_eq!(
            returned.graphics_platform().platform_identity(),
            installed_platform.platform_identity(),
            "the context handed back for storage must be the one already installed"
        );

        // A resize rebuilds from the retained target, which carries the same
        // connection structurally -- so it cannot reach a server either.
        rebuild_surface(target, 1024, 768).expect("a resize must rebuild from the retained target");
        assert_eq!(servers.opens(), 1, "a resize must not open a connection");
    }

    #[test]
    fn x11_window_from_a_foreign_server_is_refused_and_opens_nothing() {
        let mut servers = X11TestServers::new();
        let (_, stored) = attached_x11(&mut servers);
        let foreign = display(0x3000);
        servers.place(foreign, OTHER_SERVER);

        let error = build_target(x11_descriptor(foreign.as_ptr() as usize), Some(&stored)).err();

        assert_eq!(error, Some(MIGO_ERROR_INVALID_STATE));
        assert_eq!(
            servers.opens(),
            1,
            "a refused reattachment must not open a connection to the foreign server"
        );
    }

    #[test]
    fn a_stored_x11_context_refuses_a_wayland_descriptor() {
        // The other side of the guard `platform_context_rejects_display_or_kind_change`
        // covers: that one starts from Wayland, so a Wayland arm that ignored an
        // installed X11 context would still have passed it.
        let mut servers = X11TestServers::new();
        let (_, stored) = attached_x11(&mut servers);

        let error = build_target(wayland_descriptor(0x1000, 0x2000), Some(&stored)).err();

        assert_eq!(error, Some(MIGO_ERROR_INVALID_STATE));
    }
}
