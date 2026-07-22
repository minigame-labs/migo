//! Linux system-EGL presenter boundary.
//!
//! Mirrors `platform/android/presenter.rs` for the `x86_64-unknown-linux-gnu`
//! support profile. It provides a headless pbuffer path plus host-owned X11 and
//! Wayland onscreen targets through the same injection contract
//! (`EglProvider` / `EglSurfaceFactory` / `GraphicsPlatform`).
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

use super::egl_fallback;

/// System EGL shared object on glibc Linux. The unversioned `libEGL.so` symlink
/// ships only with `-dev` packages, so the runtime `.so.1` is loaded directly.
const LINUX_EGL_LIBRARY: &str = "libEGL.so.1";

/// X11 platform token, identical in `EGL_EXT_platform_x11` and
/// `EGL_KHR_platform_x11`. Not re-exported by `khronos-egl`, which carries core
/// enums only.
const EGL_PLATFORM_X11_EXT: egl::Enum = 0x31D5;
const EGL_PLATFORM_WAYLAND_EXT: egl::Enum = 0x31D8;

/// The Wayland EGL glue, loaded at run time rather than linked.
///
/// EGL cannot make a surface out of a `wl_surface` directly: it needs a
/// `wl_egl_window`, and only this library can build one. Resolving it at run
/// time keeps the SDK free of a build-time Wayland dependency, exactly as the
/// X11 path takes an opaque window token and links no X library — a host that
/// never attaches a Wayland surface never loads this.
const LINUX_WAYLAND_EGL_LIBRARY: &str = "libwayland-egl.so.1";

/// Platform-display entry points from EGL 1.5 / `EGL_EXT_platform_base`.
///
/// The engine's [`EglInstance`] is typed to **EGL 1.4** because that is the
/// floor the Android support profile can rely on: `load_required_from` resolves
/// every symbol of the requested version up front, so typing it to 1.5 would
/// refuse to load on a 1.4-only driver and break the shipping platform. The
/// 1.5 core calls are therefore unavailable through the instance, and the
/// platform entry points — resolved at runtime, exactly as GLFW/SDL/Qt do —
/// are how a specific native platform is selected without touching that floor.
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

    pub(super) fn first_resolved<F: Copy>(
        names: &[&str],
        mut resolve: impl FnMut(&str) -> Option<F>,
    ) -> Option<F> {
        names.iter().find_map(|name| resolve(name))
    }

    /// Prefer EGL 1.5 core entry points, then their EXT aliases. Both use the
    /// same calling convention for the null attribute lists used here.
    pub(super) unsafe fn first_entry_point<F: Copy>(
        egl: &EglInstance,
        names: &[&str],
    ) -> Option<F> {
        first_resolved(names, |name| unsafe { entry_point(egl, name) })
    }
}

const GET_PLATFORM_DISPLAY_NAMES: [&str; 2] = ["eglGetPlatformDisplay", "eglGetPlatformDisplayEXT"];
const CREATE_PLATFORM_WINDOW_SURFACE_NAMES: [&str; 2] = [
    "eglCreatePlatformWindowSurface",
    "eglCreatePlatformWindowSurfaceEXT",
];

fn native_platform_display(
    egl: &EglInstance,
    platform: egl::Enum,
    native_display: NonNull<c_void>,
    target: &str,
) -> EngineResult<egl::Display> {
    let platform_entry = unsafe {
        platform_ext::first_entry_point::<platform_ext::GetPlatformDisplay>(
            egl,
            &GET_PLATFORM_DISPLAY_NAMES,
        )
    };
    let entry_available = platform_entry.is_some();
    let mut platform_error = None;
    let mut legacy_error = None;
    let raw = egl_fallback::preferred_or_fallback(
        platform_entry,
        |get_platform_display| {
            // SAFETY: signature per EGL 1.5 / EXT_platform_base. The native
            // display is host-owned and a null attribute list is valid for
            // both the core EGLAttrib and EXT EGLint forms.
            let raw = unsafe {
                get_platform_display(platform, native_display.as_ptr(), std::ptr::null())
            };
            NonNull::new(raw).or_else(|| {
                // A global loader may expose a non-null stub for an unsupported
                // platform. Consume its error before trying the EGL 1.4 native
                // binding so diagnostics and the next call remain well scoped.
                platform_error = egl.get_error();
                None
            })
        },
        || {
            let display = unsafe { egl.get_display(native_display.as_ptr()) };
            display
                .map(|display| {
                    // EGL handle wrappers are guaranteed non-null on `Some`.
                    NonNull::new(display.as_ptr()).expect("EGL Display invariant")
                })
                .or_else(|| {
                    legacy_error = egl.get_error();
                    None
                })
        },
    );

    raw.map(|raw| {
        // SAFETY: non-null and produced by EGL itself.
        unsafe { egl::Display::from_ptr(raw.as_ptr()) }
    })
    .ok_or_else(|| {
        EngineError::new(ErrorCode::RenderInitializeError)
            .with_msg(format!("Linux {target} EGL display unavailable"))
            .with_detail(format!(
                "platform_entry_available={entry_available}, platform_error={platform_error:?}, legacy_error={legacy_error:?}"
            ))
    })
}

fn create_native_window_surface(
    egl: &EglInstance,
    display: egl::Display,
    config: egl::Config,
    platform_native_window: *mut c_void,
    legacy_native_window: *mut c_void,
    target: &str,
) -> EngineResult<egl::Surface> {
    let platform_entry = unsafe {
        platform_ext::first_entry_point::<platform_ext::CreatePlatformWindowSurface>(
            egl,
            &CREATE_PLATFORM_WINDOW_SURFACE_NAMES,
        )
    };
    let entry_available = platform_entry.is_some();
    let mut platform_error = None;
    let mut legacy_error = None;
    let surface = egl_fallback::preferred_or_fallback(
        platform_entry,
        |create_platform_window_surface| {
            // SAFETY: signature per EGL 1.5 / EXT_platform_base. The caller
            // supplies the platform-specific native-window representation;
            // null attributes are valid for both entry-point variants.
            let raw = unsafe {
                create_platform_window_surface(
                    display.as_ptr(),
                    config.as_ptr(),
                    platform_native_window,
                    std::ptr::null(),
                )
            };
            NonNull::new(raw)
                .map(|raw| {
                    // SAFETY: non-null and produced by EGL itself.
                    unsafe { egl::Surface::from_ptr(raw.as_ptr()) }
                })
                .or_else(|| {
                    platform_error = egl.get_error();
                    None
                })
        },
        || {
            match unsafe { egl.create_window_surface(display, config, legacy_native_window, None) }
            {
                Ok(surface) => Some(surface),
                Err(error) => {
                    // The safe wrapper has already consumed eglGetError.
                    legacy_error = Some(error);
                    None
                }
            }
        },
    );

    surface.ok_or_else(|| {
        EngineError::new(ErrorCode::RenderBackendError)
            .with_msg(format!("Linux {target} EGL window-surface creation failed"))
            .with_detail(format!(
                "platform_entry_available={entry_available}, platform_error={platform_error:?}, legacy_error={legacy_error:?}"
            ))
    })
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
    /// Onscreen Wayland: the host's `wl_display*`.
    Wayland(NonNull<c_void>),
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

    /// Onscreen provider bound to a host-owned `wl_display*`.
    pub fn wayland(display: NonNull<c_void>) -> Self {
        Self {
            target: LinuxDisplayTarget::Wayland(display),
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
            LinuxDisplayTarget::Wayland(_) => "linux-system-egl-wayland",
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
            // from the pointer. A non-null proc address can still be a loader
            // stub for an unsupported platform, so failure also falls through
            // to the EGL 1.4 native X11 binding.
            LinuxDisplayTarget::X11(display) => {
                native_platform_display(egl, EGL_PLATFORM_X11_EXT, display, "X11")
            }
            // EGL 1.4 Wayland bindings define EGLNativeDisplayType as
            // wl_display*, which is the compatibility path when the preferred
            // EGL 1.5/EXT platform call is absent or returns NO_DISPLAY.
            LinuxDisplayTarget::Wayland(display) => {
                native_platform_display(egl, EGL_PLATFORM_WAYLAND_EXT, display, "Wayland")
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
    display: NonNull<c_void>,
    window: c_ulong,
    width: u32,
    height: u32,
}

impl LinuxX11Surface {
    /// `window` is an X11 `Window` XID belonging to the host, already mapped
    /// and sized to `width` x `height` physical pixels.
    pub fn new(display: NonNull<c_void>, window: c_ulong, width: u32, height: u32) -> Self {
        Self {
            display,
            window,
            width,
            height,
        }
    }
}

// SAFETY: Display* is an opaque identity token. The host has called
// XInitThreads and keeps the connection alive through asynchronous release.
unsafe impl Send for LinuxX11Surface {}
unsafe impl Sync for LinuxX11Surface {}

impl Surface for LinuxX11Surface {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Minimal injectable Wayland EGL boundary. Safe wrappers contain all FFI and
/// let tests prove native-window ordering without a compositor.
trait WaylandEglApi: std::fmt::Debug + Send + Sync {
    fn create(&self, surface: NonNull<c_void>, width: i32, height: i32) -> Option<NonNull<c_void>>;
    fn resize(&self, window: NonNull<c_void>, width: i32, height: i32);
    fn destroy(&self, window: NonNull<c_void>);
}

/// The three `wl_egl_window` entry points, resolved once.
///
/// Declared here rather than taken from a header: the SDK carries no Wayland
/// build dependency, and these three signatures are the whole of what it needs.
mod wayland_egl {
    use super::*;
    use std::sync::{Arc, OnceLock};

    pub(super) type Create =
        unsafe extern "C" fn(*mut c_void, std::os::raw::c_int, std::os::raw::c_int) -> *mut c_void;
    pub(super) type Resize = unsafe extern "C" fn(
        *mut c_void,
        std::os::raw::c_int,
        std::os::raw::c_int,
        std::os::raw::c_int,
        std::os::raw::c_int,
    );
    pub(super) type Destroy = unsafe extern "C" fn(*mut c_void);

    pub(super) struct Glue {
        pub(super) create: Create,
        pub(super) resize: Resize,
        pub(super) destroy: Destroy,
        // Kept alive so the resolved pointers stay valid; never used directly.
        _library: libloading::Library,
    }

    // SAFETY: the three entries are plain function pointers into a library that
    // stays loaded for the process, and `libloading::Library` is itself Send +
    // Sync. Nothing here is mutated after construction.
    unsafe impl Send for Glue {}
    unsafe impl Sync for Glue {}

    impl std::fmt::Debug for Glue {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("WaylandEglGlue")
        }
    }

    impl super::WaylandEglApi for Glue {
        fn create(
            &self,
            surface: NonNull<c_void>,
            width: i32,
            height: i32,
        ) -> Option<NonNull<c_void>> {
            // SAFETY: the host keeps wl_surface alive and dimensions were
            // range-checked before this call.
            NonNull::new(unsafe { (self.create)(surface.as_ptr(), width, height) })
        }

        fn resize(&self, window: NonNull<c_void>, width: i32, height: i32) {
            // SAFETY: this Glue created the uniquely-owned window.
            unsafe { (self.resize)(window.as_ptr(), width, height, 0, 0) };
        }

        fn destroy(&self, window: NonNull<c_void>) {
            // SAFETY: the owner calls this exactly once after EGL detaches.
            unsafe { (self.destroy)(window.as_ptr()) };
        }
    }

    static GLUE: OnceLock<Option<Arc<Glue>>> = OnceLock::new();

    /// Resolve the glue, or `None` when this system has no Wayland EGL.
    ///
    /// Loaded lazily and once: a host that only ever attaches X11 or renders
    /// offscreen must not pay for, or fail on, a library it never needs.
    pub(super) fn glue() -> Option<Arc<dyn super::WaylandEglApi>> {
        GLUE.get_or_init(|| {
            let library = unsafe { libloading::Library::new(LINUX_WAYLAND_EGL_LIBRARY) }.ok()?;
            // SAFETY: the signatures above are the ones wayland-egl documents.
            unsafe {
                let create = *library.get::<Create>(b"wl_egl_window_create\0").ok()?;
                let resize = *library.get::<Resize>(b"wl_egl_window_resize\0").ok()?;
                let destroy = *library.get::<Destroy>(b"wl_egl_window_destroy\0").ok()?;
                Some(Arc::new(Glue {
                    create,
                    resize,
                    destroy,
                    _library: library,
                }))
            }
        })
        .as_ref()
        .map(|glue| Arc::clone(glue) as Arc<dyn super::WaylandEglApi>)
    }
}

/// Onscreen Wayland target. The `wl_surface` belongs to the host, which also
/// owns its role (xdg_toplevel or otherwise) and its event dispatch; this crate
/// only ever hands the pointer to `wl_egl_window_create`.
#[derive(Debug)]
pub struct LinuxWaylandSurface {
    display: NonNull<c_void>,
    surface: NonNull<c_void>,
    width: u32,
    height: u32,
}

// SAFETY: as for the X11 display handle -- an opaque token this crate passes on
// and never dereferences, so moving it between threads adds no aliasing. The
// host guarantees it outlives the attachment.
unsafe impl Send for LinuxWaylandSurface {}
unsafe impl Sync for LinuxWaylandSurface {}

impl LinuxWaylandSurface {
    /// `surface` is a host-owned `wl_surface*` already given a role and sized
    /// to `width` x `height` physical pixels.
    pub fn new(
        display: NonNull<c_void>,
        surface: NonNull<c_void>,
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            display,
            surface,
            width,
            height,
        }
    }
}

impl Surface for LinuxWaylandSurface {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

struct WaylandEglWindow {
    handle: NonNull<c_void>,
    api: Arc<dyn WaylandEglApi>,
}

unsafe impl Send for WaylandEglWindow {}
unsafe impl Sync for WaylandEglWindow {}

impl std::fmt::Debug for WaylandEglWindow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WaylandEglWindow")
            .field("handle", &self.handle)
            .finish_non_exhaustive()
    }
}

impl Drop for WaylandEglWindow {
    fn drop(&mut self) {
        self.api.destroy(self.handle);
    }
}

#[derive(Debug)]
struct WaylandPreparedState {
    width: i32,
    height: i32,
    window: Option<WaylandEglWindow>,
}

/// Non-owning Wayland identity with one lazily materialized native window.
///
/// Identity includes both `wl_display` and `wl_surface`. The mutable state is
/// cold-path only and serializes create/resize against each other; frame
/// presentation never touches this mutex.
pub struct LinuxWaylandPreparedSurface {
    display: NonNull<c_void>,
    surface: NonNull<c_void>,
    state: parking_lot::Mutex<WaylandPreparedState>,
    api: Arc<dyn WaylandEglApi>,
}

// SAFETY: pointers are opaque tokens handed to EGL/wayland-egl and mutable
// state is serialized by the cold-path mutex.
unsafe impl Send for LinuxWaylandPreparedSurface {}
unsafe impl Sync for LinuxWaylandPreparedSurface {}

impl std::fmt::Debug for LinuxWaylandPreparedSurface {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LinuxWaylandPreparedSurface")
            .field("display", &self.display)
            .field("surface", &self.surface)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl LinuxWaylandPreparedSurface {
    fn new(
        display: NonNull<c_void>,
        surface: NonNull<c_void>,
        width: u32,
        height: u32,
        api: Arc<dyn WaylandEglApi>,
    ) -> EngineResult<Self> {
        let width = i32::try_from(width).map_err(|_| {
            EngineError::new(ErrorCode::InvalidOperation)
                .with_msg("Wayland Surface width exceeds wl_egl_window range")
        })?;
        let height = i32::try_from(height).map_err(|_| {
            EngineError::new(ErrorCode::InvalidOperation)
                .with_msg("Wayland Surface height exceeds wl_egl_window range")
        })?;
        Ok(Self {
            display,
            surface,
            state: parking_lot::Mutex::new(WaylandPreparedState {
                width,
                height,
                window: None,
            }),
            api,
        })
    }

    fn materialize_locked(
        &self,
        state: &mut WaylandPreparedState,
    ) -> EngineResult<NonNull<c_void>> {
        if let Some(window) = state.window.as_ref() {
            return Ok(window.handle);
        }
        let handle = self
            .api
            .create(self.surface, state.width, state.height)
            .ok_or_else(|| {
                EngineError::new(ErrorCode::RenderInitializeError)
                    .with_msg("wl_egl_window_create failed")
            })?;
        state.window = Some(WaylandEglWindow {
            handle,
            api: Arc::clone(&self.api),
        });
        Ok(handle)
    }

    fn materialize_native_window(&self) -> EngineResult<NonNull<c_void>> {
        let mut state = self.state.lock();
        self.materialize_locked(&mut state)
    }
}

impl PreparedEglSurface for LinuxWaylandPreparedSurface {
    fn backend_id(&self) -> GraphicsBackendId {
        GraphicsBackendId::of::<LinuxSystemEglBackend>()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn same_native_surface(&self, other: &dyn PreparedEglSurface) -> bool {
        other
            .as_any()
            .downcast_ref::<LinuxWaylandPreparedSurface>()
            .is_some_and(|other| self.display == other.display && self.surface == other.surface)
    }

    fn reconfigure_from(&self, candidate: &dyn PreparedEglSurface) -> EngineResult<()> {
        let candidate = candidate
            .as_any()
            .downcast_ref::<LinuxWaylandPreparedSurface>()
            .filter(|candidate| {
                self.display == candidate.display && self.surface == candidate.surface
            })
            .ok_or_else(|| {
                EngineError::new(ErrorCode::InvalidOperation)
                    .with_msg("Wayland resize candidate has a different display or surface")
            })?;
        if std::ptr::eq(self, candidate) {
            return Ok(());
        }
        let (width, height) = {
            let candidate = candidate.state.lock();
            (candidate.width, candidate.height)
        };
        let mut installed = self.state.lock();
        if let Some(window) = installed.window.as_ref() {
            self.api.resize(window.handle, width, height);
        }
        installed.width = width;
        installed.height = height;
        Ok(())
    }

    fn create_window_surface(
        &self,
        egl: &EglInstance,
        display: egl::Display,
        config: egl::Config,
    ) -> EngineResult<egl::Surface> {
        // The platform entry point takes a *pointer to* the native window, and
        // on Wayland the native window is the `wl_egl_window` -- not the
        // `wl_surface`. Passing the surface here compiles, links, and produces
        // a surface that never presents.
        // Calling conventions differ between platforms, and getting this wrong
        // fails at surface creation rather than anywhere useful. X11's native
        // window is an XID, so the EXT entry point takes a pointer *to* it.
        // Wayland's native window is already a pointer -- the wl_egl_window --
        // so it is passed directly. Wrapping it in another pointer, as the X11
        // path correctly does for its XID, makes EGL read the stack slot
        // holding the pointer as if it were the window.
        let mut state = self.state.lock();
        let egl_window = self.materialize_locked(&mut state)?;
        // EGL 1.4 and platform entry points both take wl_egl_window* by value.
        let created = create_native_window_surface(
            egl,
            display,
            config,
            egl_window.as_ptr(),
            egl_window.as_ptr(),
            "Wayland",
        );
        if created.is_err() {
            // EGL retained nothing on failure, so destroy the failed native
            // candidate now. A later retry will materialize a fresh wrapper.
            drop(state.window.take());
        }
        created
    }
}

#[derive(Clone, Copy, Debug)]
enum LinuxSurfaceFactoryTarget {
    Offscreen,
    X11(NonNull<c_void>),
    Wayland(NonNull<c_void>),
}

unsafe impl Send for LinuxSurfaceFactoryTarget {}
unsafe impl Sync for LinuxSurfaceFactoryTarget {}

#[derive(Debug)]
pub struct LinuxEglSurfaceFactory {
    target: LinuxSurfaceFactoryTarget,
}

impl LinuxEglSurfaceFactory {
    fn offscreen() -> Self {
        Self {
            target: LinuxSurfaceFactoryTarget::Offscreen,
        }
    }

    fn x11(display: NonNull<c_void>) -> Self {
        Self {
            target: LinuxSurfaceFactoryTarget::X11(display),
        }
    }

    fn wayland(display: NonNull<c_void>) -> Self {
        Self {
            target: LinuxSurfaceFactoryTarget::Wayland(display),
        }
    }
}

impl EglSurfaceFactory for LinuxEglSurfaceFactory {
    fn backend_id(&self) -> GraphicsBackendId {
        GraphicsBackendId::of::<LinuxSystemEglBackend>()
    }

    fn prepare(&self, surface: &dyn Surface) -> EngineResult<PreparedEglSurfaceRef> {
        let any = surface.as_any();
        if let (LinuxSurfaceFactoryTarget::Offscreen, Some(offscreen)) =
            (self.target, any.downcast_ref::<LinuxOffscreenSurface>())
        {
            return Ok(Arc::new(LinuxPreparedSurface {
                width: offscreen.width,
                height: offscreen.height,
            }));
        }
        if let (LinuxSurfaceFactoryTarget::X11(display), Some(x11)) =
            (self.target, any.downcast_ref::<LinuxX11Surface>())
        {
            if display != x11.display {
                return Err(EngineError::new(ErrorCode::InvalidOperation)
                    .with_msg("X11 Surface Display does not match EGL platform Display"));
            }
            return Ok(Arc::new(LinuxX11PreparedSurface {
                display,
                window: x11.window,
            }));
        }
        if let (LinuxSurfaceFactoryTarget::Wayland(display), Some(wayland)) =
            (self.target, any.downcast_ref::<LinuxWaylandSurface>())
        {
            if display != wayland.display {
                return Err(EngineError::new(ErrorCode::InvalidOperation)
                    .with_msg("Wayland Surface display does not match EGL platform display"));
            }
            let api = wayland_egl::glue().ok_or_else(|| {
                EngineError::new(ErrorCode::Unsupported)
                    .with_msg("Wayland surface attached but the Wayland EGL glue is absent")
                    .with_detail(format!("{LINUX_WAYLAND_EGL_LIBRARY} could not be loaded"))
            })?;
            return Ok(Arc::new(LinuxWaylandPreparedSurface::new(
                display,
                wayland.surface,
                wayland.width,
                wayland.height,
                api,
            )?));
        }
        Err(EngineError::new(ErrorCode::Unsupported)
            .with_msg("Surface kind does not match the Linux EGL platform")
            .with_detail(format!("factory={:?}, surface={surface:?}", self.target)))
    }
}

/// Non-owning onscreen target. Identity is Display plus Window: XIDs are scoped
/// to a connection and cannot be compared safely without that Display.
#[derive(Debug)]
pub struct LinuxX11PreparedSurface {
    display: NonNull<c_void>,
    window: c_ulong,
}

unsafe impl Send for LinuxX11PreparedSurface {}
unsafe impl Sync for LinuxX11PreparedSurface {}

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
            .is_some_and(|other| self.display == other.display && self.window == other.window)
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
        create_native_window_surface(
            egl,
            display,
            config,
            &window as *const c_ulong as *mut c_void,
            window as *mut c_void,
            &format!("X11 window 0x{window:x}"),
        )
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
        Arc::new(LinuxEglSurfaceFactory::offscreen()),
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
        Arc::new(LinuxEglSurfaceFactory::x11(display)),
    )
}

/// Onscreen Wayland platform bound to a host-owned `wl_display*`.
///
/// The host owns the connection, the surface's role and its event dispatch;
/// this crate only hands the display to EGL. The pointer must stay valid for
/// the whole attachment.
pub fn linux_wayland_graphics_platform(display: NonNull<c_void>) -> EngineResult<GraphicsPlatform> {
    GraphicsPlatform::try_new(
        Arc::new(LinuxEglProvider::wayland(display)),
        Arc::new(LinuxEglSurfaceFactory::wayland(display)),
    )
}

#[cfg(test)]
mod tests {
    use parking_lot::Mutex;

    use super::*;

    #[test]
    fn egl15_core_entry_points_are_preferred_before_ext_aliases() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&calls);
        let resolved = platform_ext::first_resolved(&GET_PLATFORM_DISPLAY_NAMES, move |name| {
            seen.lock().push(name.to_string());
            (name == "eglGetPlatformDisplay").then_some(7_u8)
        });
        assert_eq!(resolved, Some(7));
        assert_eq!(&*calls.lock(), &["eglGetPlatformDisplay"]);

        let resolved =
            platform_ext::first_resolved(&CREATE_PLATFORM_WINDOW_SURFACE_NAMES, |name| {
                (name.ends_with("EXT")).then_some(9_u8)
            });
        assert_eq!(resolved, Some(9), "EXT remains the EGL 1.4 fallback");
    }

    #[derive(Debug)]
    struct FakeWaylandEgl {
        events: Arc<Mutex<Vec<&'static str>>>,
        window: NonNull<c_void>,
    }

    unsafe impl Send for FakeWaylandEgl {}
    unsafe impl Sync for FakeWaylandEgl {}

    impl WaylandEglApi for FakeWaylandEgl {
        fn create(
            &self,
            _surface: NonNull<c_void>,
            _width: i32,
            _height: i32,
        ) -> Option<NonNull<c_void>> {
            self.events.lock().push("native-create");
            Some(self.window)
        }

        fn resize(&self, _window: NonNull<c_void>, _width: i32, _height: i32) {
            self.events.lock().push("native-resize");
        }

        fn destroy(&self, _window: NonNull<c_void>) {
            self.events.lock().push("native-destroy");
        }
    }

    #[test]
    fn wayland_window_is_lazy_resized_in_place_and_destroyed_after_egl() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let api: Arc<dyn WaylandEglApi> = Arc::new(FakeWaylandEgl {
            events: Arc::clone(&events),
            window: NonNull::new(0x6a6a_0001usize as *mut c_void).unwrap(),
        });
        let display = NonNull::new(0x5a5a_1000usize as *mut c_void).unwrap();
        let surface = NonNull::new(0x5a5a_0001usize as *mut c_void).unwrap();
        let installed =
            LinuxWaylandPreparedSurface::new(display, surface, 320, 240, Arc::clone(&api)).unwrap();
        assert!(events.lock().is_empty(), "prepare must stay lazy");

        installed.materialize_native_window().unwrap();
        let candidate = LinuxWaylandPreparedSurface::new(display, surface, 1280, 720, api).unwrap();
        installed.reconfigure_from(&candidate).unwrap();
        drop(candidate);
        assert_eq!(&*events.lock(), &["native-create", "native-resize"]);

        // CanvasManager's successful EGL teardown happens before it drops the
        // installed PreparedEglSurface. Record that external boundary here.
        events.lock().push("egl-destroy");
        drop(installed);
        assert_eq!(
            &*events.lock(),
            &[
                "native-create",
                "native-resize",
                "egl-destroy",
                "native-destroy"
            ]
        );
    }

    /// Two Wayland surfaces are the same native surface exactly when they wrap
    /// the same `wl_surface`, so a resize keeps the attachment and a different
    /// surface never inherits it.
    ///
    /// Checked on the plain Surface because identity is a property of what the
    /// host handed over, not of the lazily-created EGL wrapper.
    #[test]
    fn wayland_surface_identity_is_the_surface_not_the_size() {
        let handle = NonNull::new(0x5a5a_0001usize as *mut c_void).expect("token");
        let other = NonNull::new(0x5a5a_0002usize as *mut c_void).expect("token");
        let display = NonNull::new(0x5a5a_1000usize as *mut c_void).expect("display");

        let small = LinuxWaylandSurface::new(display, handle, 320, 240);
        let large = LinuxWaylandSurface::new(display, handle, 1280, 720);
        let different = LinuxWaylandSurface::new(display, other, 320, 240);

        assert_eq!(small.surface, large.surface, "a resize is the same surface");
        assert_ne!(small.size(), large.size());
        assert_ne!(
            small.surface, different.surface,
            "a different wl_surface must never compare equal"
        );
    }

    /// A Wayland surface must not be mistaken for an X11 one: the factory
    /// downcasts, and an arm that matched the wrong type would hand EGL a
    /// pointer of the wrong kind.
    #[test]
    fn wayland_and_x11_surfaces_are_never_confused() {
        let display = NonNull::new(0x5a5a_1000usize as *mut c_void).expect("display");
        let wayland = LinuxWaylandSurface::new(
            display,
            NonNull::new(0x5a5a_0001usize as *mut c_void).expect("token"),
            640,
            480,
        );
        let x11 = LinuxX11Surface::new(display, 0x2a0_0001, 640, 480);

        assert!(wayland.as_any().downcast_ref::<LinuxX11Surface>().is_none());
        assert!(x11.as_any().downcast_ref::<LinuxWaylandSurface>().is_none());
    }

    #[test]
    fn wayland_platform_pairs_provider_and_factory() {
        let display = NonNull::new(0xdead_beefusize as *mut c_void).expect("token");
        let platform = linux_wayland_graphics_platform(display).expect("wayland graphics platform");
        assert_eq!(
            platform.egl_provider().backend_id(),
            platform.surface_factory().backend_id(),
        );
        assert_eq!(platform.egl_provider().label(), "linux-system-egl-wayland");
    }

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
        let prepared = LinuxEglSurfaceFactory::offscreen()
            .prepare(&surface)
            .expect("prepare offscreen");
        assert!(prepared.same_native_surface(prepared.as_ref()));
    }

    // ---- X11 onscreen target ----

    #[test]
    fn x11_surface_prepares_and_reports_its_size() {
        let display = NonNull::new(0x5a5a_1000usize as *mut c_void).expect("display");
        let surface = LinuxX11Surface::new(display, 0x2a0_0001, 800, 600);
        assert_eq!(surface.size(), (800, 600));
        let prepared = LinuxEglSurfaceFactory::x11(display)
            .prepare(&surface)
            .expect("prepare x11");
        assert!(prepared.same_native_surface(prepared.as_ref()));
    }

    #[test]
    fn x11_window_identity_is_display_plus_xid_not_size() {
        // A window keeps its identity across a resize, and two windows that
        // happen to share a size are still different surfaces. Getting this
        // wrong would let the render binding reuse a dead EGLSurface.
        let display = NonNull::new(0x5a5a_1000usize as *mut c_void).expect("display");
        let other_display = NonNull::new(0x5a5a_2000usize as *mut c_void).expect("other display");
        let prepare = |factory_display, surface_display, window, w, h| {
            LinuxEglSurfaceFactory::x11(factory_display)
                .prepare(&LinuxX11Surface::new(surface_display, window, w, h))
                .expect("prepare x11")
        };
        let resized = prepare(display, display, 0x2a0_0001, 1024, 768);
        assert!(
            prepare(display, display, 0x2a0_0001, 800, 600).same_native_surface(resized.as_ref())
        );
        assert!(
            !prepare(display, display, 0x2a0_0002, 800, 600)
                .same_native_surface(prepare(display, display, 0x2a0_0001, 800, 600).as_ref())
        );
        assert!(
            !prepare(other_display, other_display, 0x2a0_0001, 800, 600)
                .same_native_surface(resized.as_ref())
        );
    }

    #[test]
    fn offscreen_and_x11_targets_are_never_the_same_surface() {
        let display = NonNull::new(0x5a5a_1000usize as *mut c_void).expect("display");
        let offscreen = LinuxEglSurfaceFactory::offscreen()
            .prepare(&LinuxOffscreenSurface::new(800, 600))
            .expect("prepare offscreen");
        let x11 = LinuxEglSurfaceFactory::x11(display)
            .prepare(&LinuxX11Surface::new(display, 0x2a0_0001, 800, 600))
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
        let error = LinuxEglSurfaceFactory::offscreen()
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
