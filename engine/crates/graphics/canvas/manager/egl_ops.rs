extern crate khronos_egl as egl;

use egl::EGL1_4;
use libloading::Library;
use shared::error::{EngineResult, ErrorCode};

use super::types::ee;

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
    pub egl: egl::DynamicInstance<EGL1_4>,
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

/// Initialize EGL with the given library path
pub(super) fn init_egl(egl_lib_path: &str) -> EngineResult<EglInitResult> {
    let egl_lib = unsafe { Library::new(egl_lib_path) }.map_err(|e| {
        ee(
            ErrorCode::RenderBackendError,
            format!("load EGL failed: {e:?}"),
        )
    })?;

    let egl = unsafe {
        egl::DynamicInstance::<EGL1_4>::load_required_from(egl_lib).map_err(|e| {
            ee(
                ErrorCode::RenderBackendError,
                format!("egl load_required failed: {e:?}"),
            )
        })?
    };

    let display = unsafe { egl.get_display(egl::DEFAULT_DISPLAY) }
        .ok_or_else(|| ee(ErrorCode::RenderBackendError, "eglGetDisplay failed"))?;

    egl.initialize(display).map_err(|_| {
        ee(
            ErrorCode::RenderInitializeError,
            format!(
                "initialize failed: 0x{:x}",
                egl.get_error().map(|e| e as u32).unwrap_or(0)
            ),
        )
    })?;

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

    Ok(EglInitResult {
        egl,
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

    Ok((ctx, surf))
}

/// Create a window surface
pub(super) fn create_window_surface(
    egl: &egl::DynamicInstance<EGL1_4>,
    display: egl::Display,
    config: egl::Config,
    window: usize,
) -> EngineResult<egl::Surface> {
    let native_win = window as egl::NativeWindowType;
    unsafe {
        egl.create_window_surface(display, config, native_win, None)
            .map_err(|e| {
                ee(
                    ErrorCode::RenderBackendError,
                    format!("eglCreateWindowSurface failed: {e:?}"),
                )
            })
    }
}
