//! Desktop (Linux) system-EGL presenter boundary.
//!
//! Mirrors `platform/android/presenter.rs` for the `x86_64-unknown-linux-gnu`
//! support profile, but targets **offscreen (pbuffer)** rendering: a headless
//! player/CI render path that needs no window server. The onscreen X11/Wayland
//! Presenter is a later addition; the injection contract
//! (`EglProvider` / `EglSurfaceFactory` / `GraphicsPlatform`) is identical.
//!
//! NOTE: the chosen `egl::Config` must advertise `EGL_PBUFFER_BIT` in its
//! surface type for `create_pbuffer_surface` to succeed. The graphics EGL
//! manager owns config selection; onscreen-only configs will reject the
//! pbuffer path.

use std::{
    any::Any,
    ffi::{c_ulong, c_void},
    ptr::NonNull,
    sync::Arc,
};

use graphics::egl_platform::{
    EglInstance, EglProvider, EglSurfaceFactory, GraphicsBackendId, GraphicsPlatform,
    PreparedEglSurface, PreparedEglSurfaceRef,
};
use khronos_egl as egl;
use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    surface::Surface,
};

/// System EGL shared object on glibc Linux. The unversioned `libEGL.so` symlink
/// ships only with `-dev` packages, so the runtime `.so.1` is loaded directly.
const LINUX_EGL_LIBRARY: &str = "libEGL.so.1";

/// X11 platform token, identical in `EGL_EXT_platform_x11` and
/// `EGL_KHR_platform_x11`. Not re-exported by `khronos-egl`, which carries core
/// enums only.
const EGL_PLATFORM_X11_EXT: egl::Enum = 0x31D5;

/// Platform-display entry points from `EGL_EXT_platform_base`.
///
/// The engine's [`EglInstance`] is typed to **EGL 1.4** because that is the
/// floor the Android support profile can rely on: `load_required_from` resolves
/// every symbol of the requested version up front, so typing it to 1.5 would
/// refuse to load on a 1.4-only driver and break the shipping platform. The
/// 1.5 core calls are therefore unavailable through the instance, and the
/// `EXT` entry points — resolved at runtime, exactly as GLFW/SDL/Qt do — are
/// how an X11 platform display is obtained without touching that floor.
///
/// When the extension is absent the caller falls back to the legacy calls,
/// which let the driver infer the platform from the pointer.
mod platform_ext {
    use super::*;

    pub(super) type GetPlatformDisplay =
        unsafe extern "system" fn(egl::Enum, *mut c_void, *const egl::Int) -> *mut c_void;

    pub(super) type CreatePlatformWindowSurface = unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *const egl::Int,
    ) -> *mut c_void;

    /// Resolve an extension entry point, or `None` when the driver lacks it.
    ///
    /// # Safety
    /// The caller must instantiate `F` with the signature the EGL specification
    /// gives for `name`.
    pub(super) unsafe fn entry_point<F: Copy>(egl: &EglInstance, name: &str) -> Option<F> {
        // A function pointer is pointer-sized; the transmute below reinterprets
        // it as the specified prototype, which is the documented way to use
        // eglGetProcAddress.
        const _: () = assert!(size_of::<extern "system" fn()>() == size_of::<*const c_void>());
        egl.get_proc_address(name)
            .map(|pointer| unsafe { std::mem::transmute_copy::<_, F>(&pointer) })
    }
}

#[derive(Debug)]
struct LinuxSystemEglBackend;

/// Which native display the provider binds EGL to.
///
/// The X11 variant carries a handle the **host** owns. It is only ever handed
/// to `eglGetPlatformDisplay`; this crate never dereferences it, and never
/// opens or closes the connection.
#[derive(Debug, Clone, Copy)]
enum LinuxDisplayTarget {
    /// Headless: the default display, no window server involved.
    Offscreen,
    /// Onscreen X11: the host's `Display*`.
    X11(NonNull<c_void>),
}

// SAFETY: the pointer is an opaque token passed to EGL, never dereferenced
// here, so moving it between threads adds no aliasing of its own. The render
// thread resolves the display, while the host opened it on another thread —
// which is sound because the host guarantees (documented on
// `linux_x11_graphics_platform`) that it called `XInitThreads` and keeps the
// connection alive for the whole session.
unsafe impl Send for LinuxDisplayTarget {}
unsafe impl Sync for LinuxDisplayTarget {}

#[derive(Debug)]
pub struct LinuxEglProvider {
    target: LinuxDisplayTarget,
}

impl Default for LinuxEglProvider {
    fn default() -> Self {
        Self {
            target: LinuxDisplayTarget::Offscreen,
        }
    }
}

impl LinuxEglProvider {
    /// Headless provider: EGL on the default display.
    pub fn offscreen() -> Self {
        Self::default()
    }

    /// Onscreen provider bound to a host-owned X11 `Display*`.
    pub fn x11(display: NonNull<c_void>) -> Self {
        Self {
            target: LinuxDisplayTarget::X11(display),
        }
    }
}

impl EglProvider for LinuxEglProvider {
    fn backend_id(&self) -> GraphicsBackendId {
        GraphicsBackendId::of::<LinuxSystemEglBackend>()
    }

    fn label(&self) -> &str {
        match self.target {
            LinuxDisplayTarget::Offscreen => "linux-system-egl",
            LinuxDisplayTarget::X11(_) => "linux-system-egl-x11",
        }
    }

    fn load(&self) -> EngineResult<EglInstance> {
        let library = unsafe { libloading::Library::new(LINUX_EGL_LIBRARY) }.map_err(|error| {
            EngineError::new(ErrorCode::RenderBackendError)
                .with_msg("load Linux system EGL failed")
                .with_detail(format!("{LINUX_EGL_LIBRARY}: {error}"))
        })?;
        unsafe { EglInstance::load_required_from(library) }.map_err(|error| {
            EngineError::new(ErrorCode::RenderBackendError)
                .with_msg("resolve required Linux EGL symbols failed")
                .with_detail(format!("provider={}: {error:?}", self.label()))
        })
    }

    fn display(&self, egl: &EglInstance) -> EngineResult<egl::Display> {
        match self.target {
            LinuxDisplayTarget::Offscreen => unsafe { egl.get_display(egl::DEFAULT_DISPLAY) }
                .ok_or_else(|| {
                    EngineError::new(ErrorCode::RenderInitializeError)
                        .with_msg("Linux eglGetDisplay failed")
                        .with_detail(format!("provider={}", self.label()))
                }),
            // Naming the platform explicitly beats letting the driver infer it
            // from the pointer, so the EXT entry point is preferred and the
            // legacy call is only a fallback for drivers without it.
            LinuxDisplayTarget::X11(display) => {
                let raw = match unsafe {
                    platform_ext::entry_point::<platform_ext::GetPlatformDisplay>(
                        egl,
                        "eglGetPlatformDisplayEXT",
                    )
                } {
                    // SAFETY: signature per EGL_EXT_platform_base; the display
                    // pointer is the host's, kept alive per this module's
                    // contract; a null attribute list means "no attributes".
                    Some(get_platform_display) => unsafe {
                        get_platform_display(
                            EGL_PLATFORM_X11_EXT,
                            display.as_ptr(),
                            std::ptr::null(),
                        )
                    },
                    // SAFETY: legacy eglGetDisplay accepts the native display
                    // handle directly on X11.
                    None => match unsafe { egl.get_display(display.as_ptr()) } {
                        Some(display) => display.as_ptr(),
                        None => std::ptr::null_mut(),
                    },
                };
                NonNull::new(raw)
                    // SAFETY: non-null and produced by EGL itself.
                    .map(|raw| unsafe { egl::Display::from_ptr(raw.as_ptr()) })
                    .ok_or_else(|| {
                        EngineError::new(ErrorCode::RenderInitializeError)
                            .with_msg("Linux X11 EGL display unavailable")
                            .with_detail(format!("provider={}", self.label()))
                    })
            }
        }
    }
}

/// Offscreen render target for headless Linux (pbuffer-backed). Carries only
/// the physical framebuffer size the presenter allocates the pbuffer to.
#[derive(Debug)]
pub struct LinuxOffscreenSurface {
    width: u32,
    height: u32,
}

impl LinuxOffscreenSurface {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl Surface for LinuxOffscreenSurface {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Onscreen X11 render target. Carries the host's window and the physical size
/// the host mapped it at.
///
/// Both the `Window` and the `Display*` the platform was built with belong to
/// the host: it creates them, resizes them and destroys them. The engine only
/// renders into them, which is what keeps §7.2's "the SDK does not own the
/// window" rule true in code.
#[derive(Debug)]
pub struct LinuxX11Surface {
    window: c_ulong,
    width: u32,
    height: u32,
}

impl LinuxX11Surface {
    /// `window` is an X11 `Window` XID belonging to the host, already mapped
    /// and sized to `width` x `height` physical pixels.
    pub fn new(window: c_ulong, width: u32, height: u32) -> Self {
        Self {
            window,
            width,
            height,
        }
    }
}

impl Surface for LinuxX11Surface {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

#[derive(Debug, Default)]
pub struct LinuxEglSurfaceFactory;

impl EglSurfaceFactory for LinuxEglSurfaceFactory {
    fn backend_id(&self) -> GraphicsBackendId {
        GraphicsBackendId::of::<LinuxSystemEglBackend>()
    }

    fn prepare(&self, surface: &dyn Surface) -> EngineResult<PreparedEglSurfaceRef> {
        let any = surface.as_any();
        if let Some(offscreen) = any.downcast_ref::<LinuxOffscreenSurface>() {
            return Ok(Arc::new(LinuxPreparedSurface {
                width: offscreen.width,
                height: offscreen.height,
            }));
        }
        if let Some(x11) = any.downcast_ref::<LinuxX11Surface>() {
            return Ok(Arc::new(LinuxX11PreparedSurface { window: x11.window }));
        }
        Err(EngineError::new(ErrorCode::Unsupported)
            .with_msg("Linux presenter requires LinuxOffscreenSurface or LinuxX11Surface")
            .with_detail(format!("surface={surface:?}")))
    }
}

/// Non-owning onscreen target. Identity is the window XID, so a resized window
/// stays the same native surface while a different window never compares equal.
#[derive(Debug)]
pub struct LinuxX11PreparedSurface {
    window: c_ulong,
}

impl PreparedEglSurface for LinuxX11PreparedSurface {
    fn backend_id(&self) -> GraphicsBackendId {
        GraphicsBackendId::of::<LinuxSystemEglBackend>()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn same_native_surface(&self, other: &dyn PreparedEglSurface) -> bool {
        other
            .as_any()
            .downcast_ref::<LinuxX11PreparedSurface>()
            .is_some_and(|other| self.window == other.window)
    }

    fn create_window_surface(
        &self,
        egl: &EglInstance,
        display: egl::Display,
        config: egl::Config,
    ) -> EngineResult<egl::Surface> {
        // Mind the calling conventions: the platform entry point takes a
        // *pointer to* the native window, while legacy `eglCreateWindowSurface`
        // takes the XID by value. Passing one where the other is expected reads
        // the XID as an address.
        let window = self.window;
        let raw = match unsafe {
            platform_ext::entry_point::<platform_ext::CreatePlatformWindowSurface>(
                egl,
                "eglCreatePlatformWindowSurfaceEXT",
            )
        } {
            // SAFETY: signature per EGL_EXT_platform_base. `window` outlives the
            // call, and the host owns the underlying X11 window for the whole
            // session; a null attribute list means "no attributes".
            Some(create_platform_window_surface) => unsafe {
                create_platform_window_surface(
                    display.as_ptr(),
                    config.as_ptr(),
                    &window as *const c_ulong as *mut c_void,
                    std::ptr::null(),
                )
            },
            // SAFETY: legacy path takes the XID by value in the pointer-sized
            // native-window slot, which is the X11 convention.
            None => match unsafe {
                egl.create_window_surface(display, config, window as *mut c_void, None)
            } {
                Ok(surface) => surface.as_ptr(),
                Err(error) => {
                    return Err(EngineError::new(ErrorCode::RenderBackendError)
                        .with_msg("Linux eglCreateWindowSurface failed")
                        .with_detail(format!("window=0x{window:x}: {error:?}")));
                }
            },
        };
        NonNull::new(raw)
            // SAFETY: non-null and produced by EGL itself.
            .map(|raw| unsafe { egl::Surface::from_ptr(raw.as_ptr()) })
            .ok_or_else(|| {
                EngineError::new(ErrorCode::RenderBackendError)
                    .with_msg("Linux eglCreatePlatformWindowSurfaceEXT failed")
                    .with_detail(format!("window=0x{window:x}"))
            })
    }
}

/// Non-owning, repeatable offscreen target. A pbuffer has no native handle, so
/// identity is the (width, height) it was prepared for.
#[derive(Debug)]
pub struct LinuxPreparedSurface {
    width: u32,
    height: u32,
}

impl PreparedEglSurface for LinuxPreparedSurface {
    fn backend_id(&self) -> GraphicsBackendId {
        GraphicsBackendId::of::<LinuxSystemEglBackend>()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn same_native_surface(&self, other: &dyn PreparedEglSurface) -> bool {
        other
            .as_any()
            .downcast_ref::<LinuxPreparedSurface>()
            .is_some_and(|other| self.width == other.width && self.height == other.height)
    }

    fn create_window_surface(
        &self,
        egl: &EglInstance,
        display: egl::Display,
        config: egl::Config,
    ) -> EngineResult<egl::Surface> {
        // Offscreen: the "window surface" the render binding requests is served
        // by a pbuffer sized to the target, so no window server is needed.
        let attributes = [
            egl::WIDTH,
            self.width as egl::Int,
            egl::HEIGHT,
            self.height as egl::Int,
            egl::NONE,
        ];
        egl.create_pbuffer_surface(display, config, &attributes)
            .map_err(|error| {
                EngineError::new(ErrorCode::RenderBackendError)
                    .with_msg("Linux eglCreatePbufferSurface failed")
                    .with_detail(format!("{error:?}"))
            })
    }
}

/// Offscreen Linux graphics platform: system EGL + pbuffer surface factory.
pub fn linux_graphics_platform() -> EngineResult<GraphicsPlatform> {
    GraphicsPlatform::try_new(
        Arc::new(LinuxEglProvider::offscreen()),
        Arc::new(LinuxEglSurfaceFactory),
    )
}

/// Onscreen Linux graphics platform bound to a host-owned X11 connection.
///
/// The caller keeps ownership of the display: it must stay open for the whole
/// engine session, and must have been opened after `XInitThreads`, because the
/// render thread resolves the EGL display from it while the host services the
/// window on another thread.
pub fn linux_x11_graphics_platform(display: NonNull<c_void>) -> EngineResult<GraphicsPlatform> {
    GraphicsPlatform::try_new(
        Arc::new(LinuxEglProvider::x11(display)),
        Arc::new(LinuxEglSurfaceFactory),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_and_factory_share_backend_id() {
        // GraphicsPlatform::try_new fails closed on mismatched backend ids;
        // constructing it proves the Linux provider/factory are paired.
        let platform = linux_graphics_platform().expect("linux graphics platform");
        assert_eq!(
            platform.egl_provider().backend_id(),
            platform.surface_factory().backend_id(),
        );
        assert_eq!(platform.egl_provider().label(), "linux-system-egl");
    }

    #[test]
    fn offscreen_surface_reports_its_size() {
        let surface = LinuxOffscreenSurface::new(320, 240);
        assert_eq!(surface.size(), (320, 240));
        let prepared = LinuxEglSurfaceFactory
            .prepare(&surface)
            .expect("prepare offscreen");
        assert!(prepared.same_native_surface(prepared.as_ref()));
    }

    // ---- X11 onscreen target ----

    #[test]
    fn x11_surface_prepares_and_reports_its_size() {
        let surface = LinuxX11Surface::new(0x2a0_0001, 800, 600);
        assert_eq!(surface.size(), (800, 600));
        let prepared = LinuxEglSurfaceFactory
            .prepare(&surface)
            .expect("prepare x11");
        assert!(prepared.same_native_surface(prepared.as_ref()));
    }

    #[test]
    fn x11_window_identity_is_the_xid_not_the_size() {
        // A window keeps its identity across a resize, and two windows that
        // happen to share a size are still different surfaces. Getting this
        // wrong would let the render binding reuse a dead EGLSurface.
        let prepare = |window, w, h| {
            LinuxEglSurfaceFactory
                .prepare(&LinuxX11Surface::new(window, w, h))
                .expect("prepare x11")
        };
        let resized = prepare(0x2a0_0001, 1024, 768);
        assert!(prepare(0x2a0_0001, 800, 600).same_native_surface(resized.as_ref()));
        assert!(
            !prepare(0x2a0_0002, 800, 600)
                .same_native_surface(prepare(0x2a0_0001, 800, 600).as_ref())
        );
    }

    #[test]
    fn offscreen_and_x11_targets_are_never_the_same_surface() {
        let offscreen = LinuxEglSurfaceFactory
            .prepare(&LinuxOffscreenSurface::new(800, 600))
            .expect("prepare offscreen");
        let x11 = LinuxEglSurfaceFactory
            .prepare(&LinuxX11Surface::new(0x2a0_0001, 800, 600))
            .expect("prepare x11");
        assert!(!offscreen.same_native_surface(x11.as_ref()));
        assert!(!x11.same_native_surface(offscreen.as_ref()));
    }

    #[test]
    fn foreign_surface_types_still_fail_closed() {
        // The factory must reject anything it does not own rather than guess a
        // native handle out of it.
        #[derive(Debug)]
        struct ForeignSurface;
        impl Surface for ForeignSurface {
            fn as_any(&self) -> &dyn Any {
                self
            }
            fn size(&self) -> (u32, u32) {
                (1, 1)
            }
        }
        let error = LinuxEglSurfaceFactory
            .prepare(&ForeignSurface)
            .expect_err("foreign surface must be rejected");
        assert_eq!(error.code, ErrorCode::Unsupported);
    }

    #[test]
    fn x11_platform_pairs_provider_and_factory() {
        // Same fail-closed pairing check as the offscreen platform: a mismatched
        // backend id would make GraphicsPlatform::try_new refuse.
        let display = NonNull::new(0xdead_beef_usize as *mut c_void).expect("non-null");
        let platform = linux_x11_graphics_platform(display).expect("x11 graphics platform");
        assert_eq!(
            platform.egl_provider().backend_id(),
            platform.surface_factory().backend_id(),
        );
        assert_eq!(platform.egl_provider().label(), "linux-system-egl-x11");
    }
}
