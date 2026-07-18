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

use std::{any::Any, sync::Arc};

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

#[derive(Debug)]
struct LinuxSystemEglBackend;

#[derive(Debug, Default)]
pub struct LinuxEglProvider;

impl EglProvider for LinuxEglProvider {
    fn backend_id(&self) -> GraphicsBackendId {
        GraphicsBackendId::of::<LinuxSystemEglBackend>()
    }

    fn label(&self) -> &str {
        "linux-system-egl"
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
        unsafe { egl.get_display(egl::DEFAULT_DISPLAY) }.ok_or_else(|| {
            EngineError::new(ErrorCode::RenderInitializeError)
                .with_msg("Linux eglGetDisplay failed")
                .with_detail(format!("provider={}", self.label()))
        })
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

#[derive(Debug, Default)]
pub struct LinuxEglSurfaceFactory;

impl EglSurfaceFactory for LinuxEglSurfaceFactory {
    fn backend_id(&self) -> GraphicsBackendId {
        GraphicsBackendId::of::<LinuxSystemEglBackend>()
    }

    fn prepare(&self, surface: &dyn Surface) -> EngineResult<PreparedEglSurfaceRef> {
        let offscreen = surface
            .as_any()
            .downcast_ref::<LinuxOffscreenSurface>()
            .ok_or_else(|| {
                EngineError::new(ErrorCode::Unsupported)
                    .with_msg("Linux presenter requires LinuxOffscreenSurface")
                    .with_detail(format!("surface={surface:?}"))
            })?;
        Ok(Arc::new(LinuxPreparedSurface {
            width: offscreen.width,
            height: offscreen.height,
        }))
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
    GraphicsPlatform::try_new(Arc::new(LinuxEglProvider), Arc::new(LinuxEglSurfaceFactory))
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
}
