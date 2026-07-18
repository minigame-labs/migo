extern crate khronos_egl as egl;

use egl::EGL1_4;
use shared::error::{EngineResult, ErrorCode};

use crate::egl_platform::{EglInstance, EglProvider};

use super::types::ee;

/// Terminates an initialized display if configuration/probing returns early.
/// The guard borrows the instance so it cannot outlive the exact dispatch
/// table that initialized the display.
struct InitializedDisplayGuard<'a> {
    egl: &'a EglInstance,
    display: egl::Display,
    armed: bool,
}

impl<'a> InitializedDisplayGuard<'a> {
    fn new(egl: &'a EglInstance, display: egl::Display) -> Self {
        Self {
            egl,
            display,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for InitializedDisplayGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.egl.terminate(self.display);
        }
    }
}

/// Owns one initialized EGLDisplay and the resource context that roots its
/// share group.  It is installed in CanvasManager immediately after init so
/// every constructor error and panic has the same best-effort EGL fallback.
pub(super) struct EglRuntime {
    instance: EglInstance,
    display: egl::Display,
    resource: Option<(egl::Context, egl::Surface)>,
    initialized: bool,
}

impl EglRuntime {
    fn new(instance: EglInstance, display: egl::Display) -> Self {
        Self {
            instance,
            display,
            resource: None,
            initialized: true,
        }
    }

    pub(super) fn track_resource(&mut self, context: egl::Context, surface: egl::Surface) {
        assert!(
            self.resource.is_none(),
            "EGL resource owner cannot track two share-group roots"
        );
        self.resource = Some((context, surface));
    }

    pub(super) fn untrack_resource(&mut self) -> Option<(egl::Context, egl::Surface)> {
        self.resource.take()
    }

    /// Idempotent final display teardown. Callers must release every window
    /// and offscreen surface first; this method owns only the root pbuffer and
    /// the final eglTerminate authority.
    pub(super) fn shutdown(&mut self) {
        if !self.initialized {
            return;
        }

        // Disarm before calling driver code, and isolate each wrapper call.
        // khronos-egl normally returns Result, but a non-conforming driver that
        // reports EGL_FALSE without an EGL error can still trigger an internal
        // unwrap. Drop must never double-panic during render-thread unwinding.
        self.initialized = false;
        let resource = self.resource.take();
        let instance = &self.instance;
        let display = self.display;
        let ignore_driver_panic = |operation: &mut dyn FnMut()| {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation));
        };

        ignore_driver_panic(&mut || {
            let _ = instance.make_current(display, None, None, None);
        });
        if let Some((context, surface)) = resource {
            ignore_driver_panic(&mut || {
                let _ = instance.destroy_surface(display, surface);
            });
            ignore_driver_panic(&mut || {
                let _ = instance.destroy_context(display, context);
            });
        }
        ignore_driver_panic(&mut || {
            let _ = instance.terminate(display);
        });
    }
}

impl std::ops::Deref for EglRuntime {
    type Target = EglInstance;

    fn deref(&self) -> &Self::Target {
        &self.instance
    }
}

impl Drop for EglRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Destroys a newly-created context if pbuffer creation (or unwinding between
/// the two EGL calls) does not complete the pair.
struct ContextCleanupGuard<'a> {
    egl: &'a EglInstance,
    display: egl::Display,
    context: Option<egl::Context>,
}

impl<'a> ContextCleanupGuard<'a> {
    fn new(egl: &'a EglInstance, display: egl::Display, context: egl::Context) -> Self {
        Self {
            egl,
            display,
            context: Some(context),
        }
    }

    fn disarm(mut self) -> egl::Context {
        self.context
            .take()
            .expect("pbuffer context cleanup guard must be armed")
    }
}

impl Drop for ContextCleanupGuard<'_> {
    fn drop(&mut self) {
        if let Some(context) = self.context.take() {
            let _ = self.egl.destroy_context(self.display, context);
        }
    }
}

// ---- EGL_EXT_create_context_robustness ----
//
// Lets Migo request a GL ES context that the driver observes
// periodically for resets.  Paired with `GL_KHR_robustness` it
// turns a slow "invisible corruption → caught only at next
// eglSwapBuffers" failure into "every GL call starts returning
// GL_CONTEXT_LOST and glGetGraphicsResetStatus reports the
// reason" — giving the render thread a chance to tear down and
// rebuild before the user sees a black frame.  See wgpu-hal's
// GLES backend for the same pattern
// (`wgpu-hal/src/gles/egl.rs:511-567`).
const EGL_CONTEXT_OPENGL_RESET_NOTIFICATION_STRATEGY_EXT: egl::Int = 0x3138;
const EGL_LOSE_CONTEXT_ON_RESET_EXT: egl::Int = 0x31BF;

/// EGL initialization result
pub(super) struct EglInitResult {
    pub egl: EglRuntime,
    pub display: egl::Display,
    pub config: egl::Config,
    /// The GLES version actually negotiated (3 = ES 3.0+, 2 = ES 2.0 fallback).
    pub gles_major: u32,
    /// Whether the driver supports `EGL_EXT_create_context_robustness` so
    /// subsequent `create_context` calls can add the reset-notification
    /// strategy attribute (R-3).  Cached here rather than re-queried
    /// per context to avoid the extra `eglQueryString` on every canvas
    /// create.
    pub has_robust_context: bool,
}

/// Initialize EGL exclusively through the platform-selected provider.
pub(super) fn init_egl(provider: &dyn EglProvider) -> EngineResult<EglInitResult> {
    let egl = provider.load()?;
    let display = provider.display(&egl)?;

    egl.initialize(display).map_err(|_| {
        ee(
            ErrorCode::RenderInitializeError,
            format!(
                "initialize failed: 0x{:x}",
                egl.get_error().map(|e| e as u32).unwrap_or(0)
            ),
        )
    })?;
    let mut initialized_display = InitializedDisplayGuard::new(&egl, display);

    // EGL_OPENGL_ES3_BIT_KHR — request configs that support ES 3.0.
    // Defined by EGL_KHR_create_context, widely available on Android 5.0+.
    const OPENGL_ES3_BIT: egl::Int = 0x0040;

    // Try ES 3.0-capable configs first, then fall back to ES 2.0.
    // Within each GLES level, try depth/stencil variants in preference order.
    struct ConfigCandidate {
        depth: egl::Int,
        stencil: egl::Int,
        renderable: egl::Int,
        gles_major: u32,
    }

    let candidates = [
        // ES 3.0 — primary: D16+S8
        ConfigCandidate {
            depth: 16,
            stencil: 8,
            renderable: OPENGL_ES3_BIT,
            gles_major: 3,
        },
        // ES 3.0 — fallback: D24+S8 (some drivers only offer D24S8 packed)
        ConfigCandidate {
            depth: 24,
            stencil: 8,
            renderable: OPENGL_ES3_BIT,
            gles_major: 3,
        },
        // ES 3.0 — fallback: D16 no stencil
        ConfigCandidate {
            depth: 16,
            stencil: 0,
            renderable: OPENGL_ES3_BIT,
            gles_major: 3,
        },
        // ES 2.0 — primary: D16+S8
        ConfigCandidate {
            depth: 16,
            stencil: 8,
            renderable: egl::OPENGL_ES2_BIT,
            gles_major: 2,
        },
        // ES 2.0 — fallback: D24+S8
        ConfigCandidate {
            depth: 24,
            stencil: 8,
            renderable: egl::OPENGL_ES2_BIT,
            gles_major: 2,
        },
        // ES 2.0 — fallback: D16 no stencil
        ConfigCandidate {
            depth: 16,
            stencil: 0,
            renderable: egl::OPENGL_ES2_BIT,
            gles_major: 2,
        },
    ];

    let mut config = None;
    let mut gles_major = 2u32;
    for c in &candidates {
        let attrs = [
            egl::RED_SIZE,
            8,
            egl::GREEN_SIZE,
            8,
            egl::BLUE_SIZE,
            8,
            egl::ALPHA_SIZE,
            8,
            egl::DEPTH_SIZE,
            c.depth,
            egl::STENCIL_SIZE,
            c.stencil,
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT | egl::PBUFFER_BIT,
            egl::RENDERABLE_TYPE,
            c.renderable,
            egl::NONE,
        ];
        if let Ok(Some(cfg)) = egl.choose_first_config(display, &attrs) {
            config = Some(cfg);
            gles_major = c.gles_major;
            break;
        }
    }
    let config = config.ok_or_else(|| {
        ee(
            ErrorCode::RenderChooseConfigError,
            "all EGL config candidates failed",
        )
    })?;

    tracing::info!("EGL config selected: GLES {gles_major}.0");

    // Probe `EGL_EXT_create_context_robustness` once at init.  The
    // robustness context attribute is harmless on drivers that
    // advertise the extension but a hard-error on those that
    // don't — keep the toggle here so `create_pbuffer_context` /
    // `create_onscreen` can decide whether to include it.
    let extensions = egl
        .query_string(Some(display), egl::EXTENSIONS)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let has_robust_context = extensions.contains("EGL_EXT_create_context_robustness");
    if has_robust_context {
        tracing::info!(
            "EGL_EXT_create_context_robustness available; GL reset notifications enabled"
        );
    }

    initialized_display.disarm();
    drop(initialized_display);

    Ok(EglInitResult {
        egl: EglRuntime::new(egl, display),
        display,
        config,
        gles_major,
        has_robust_context,
    })
}

/// Build the `eglCreateContext` attribute list for a GLES
/// context, optionally appending the robustness / reset
/// notification attributes when the driver supports them (R-3).
pub(super) fn build_ctx_attribs(gles_major: u32, has_robust_context: bool) -> Vec<egl::Int> {
    let mut attribs: Vec<egl::Int> = vec![
        egl::CONTEXT_CLIENT_VERSION as egl::Int,
        gles_major as egl::Int,
    ];
    if has_robust_context {
        // Asking for LoseContextOnReset means the driver will
        // transition the context to LOST status on a reset, and
        // `glGetGraphicsResetStatus` returns a non-zero reason
        // that the render loop can act on.  An implementation
        // that doesn't support robustness at all would have
        // failed the `has_robust_context` probe; drivers that
        // advertise it but don't honour the strategy simply
        // ignore the attribute, which is the desired fallback
        // behaviour.
        attribs.push(EGL_CONTEXT_OPENGL_RESET_NOTIFICATION_STRATEGY_EXT);
        attribs.push(EGL_LOSE_CONTEXT_ON_RESET_EXT);
    }
    attribs.push(egl::NONE as egl::Int);
    attribs
}

/// Create a pbuffer context with optional share context.
///
/// `gles_major` should match the version negotiated by `init_egl` so all
/// contexts in the share group use the same GLES level.
pub(super) fn create_pbuffer_context(
    egl: &egl::DynamicInstance<EGL1_4>,
    display: egl::Display,
    config: egl::Config,
    share: Option<egl::Context>,
    width: u32,
    height: u32,
    gles_major: u32,
    has_robust_context: bool,
) -> EngineResult<(egl::Context, egl::Surface)> {
    let ctx_attribs = build_ctx_attribs(gles_major, has_robust_context);

    egl.bind_api(egl::OPENGL_ES_API).map_err(|e| {
        ee(
            ErrorCode::RenderBackendError,
            format!("eglBindAPI failed: {e:?}"),
        )
    })?;

    let ctx = egl
        .create_context(display, config, share, &ctx_attribs)
        .map_err(|e| {
            ee(
                ErrorCode::RenderBackendError,
                format!("eglCreateContext failed: {e:?}"),
            )
        })?;
    let context_cleanup = ContextCleanupGuard::new(egl, display, ctx);

    let pbuf_attribs = [
        egl::WIDTH as i32,
        width as i32,
        egl::HEIGHT as i32,
        height as i32,
        egl::NONE as i32,
    ];
    let surf = egl
        .create_pbuffer_surface(display, config, &pbuf_attribs)
        .map_err(|e| {
            ee(
                ErrorCode::RenderBackendError,
                format!("eglCreatePbufferSurface failed: {e:?}"),
            )
        })?;
    let ctx = context_cleanup.disarm();

    Ok((ctx, surf))
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use crate::egl_platform::{EglInstance, EglProvider, GraphicsBackendId};
    use shared::error::{EngineError, EngineResult, ErrorCode};

    use super::init_egl;

    const EGL_OPS_SOURCE: &str = include_str!("egl_ops.rs");

    #[derive(Debug)]
    struct InjectedFailureProvider {
        loads: Arc<AtomicUsize>,
    }

    impl EglProvider for InjectedFailureProvider {
        fn backend_id(&self) -> GraphicsBackendId {
            GraphicsBackendId::of::<Self>()
        }

        fn label(&self) -> &str {
            "sentinel-injected-egl"
        }

        fn load(&self) -> EngineResult<EglInstance> {
            self.loads.fetch_add(1, Ordering::Relaxed);
            Err(EngineError::new(ErrorCode::RenderInitializeError)
                .with_msg("sentinel provider load failure"))
        }

        fn display(&self, _egl: &EglInstance) -> EngineResult<khronos_egl::Display> {
            panic!("display must not be requested after provider load failure")
        }
    }

    #[test]
    fn injected_egl_provider_is_the_only_loader_used_by_init() {
        let loads = Arc::new(AtomicUsize::new(0));
        let provider = InjectedFailureProvider {
            loads: Arc::clone(&loads),
        };

        let error = init_egl(&provider).err().expect("provider must fail");
        assert_eq!(loads.load(Ordering::Relaxed), 1);
        assert_eq!(error.msg, "sentinel provider load failure");
    }

    #[test]
    fn initialized_display_and_partial_pbuffer_creation_have_raii_guards() {
        assert!(EGL_OPS_SOURCE.contains("struct InitializedDisplayGuard"));
        assert!(EGL_OPS_SOURCE.contains("struct ContextCleanupGuard"));

        let init = EGL_OPS_SOURCE
            .split("pub(super) fn init_egl")
            .nth(1)
            .expect("init_egl must exist");
        assert!(init.contains("InitializedDisplayGuard::new"));

        let pbuffer = EGL_OPS_SOURCE
            .split("pub(super) fn create_pbuffer_context")
            .nth(1)
            .expect("create_pbuffer_context must exist");
        assert!(pbuffer.contains("ContextCleanupGuard::new"));
    }
}
