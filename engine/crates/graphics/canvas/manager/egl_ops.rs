extern crate khronos_egl as egl;

use egl::EGL1_4;
use libloading::Library;
use shared::error::{EngineResult, ErrorCode};

use super::types::ee;

/// EGL initialization result
pub(super) struct EglInitResult {
    pub egl: egl::DynamicInstance<EGL1_4>,
    pub display: egl::Display,
    pub config: egl::Config,
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

    // Try configs in preference order; fall back to simpler configs on
    // devices that don't support the full RGBA8+D16+S8 combination.
    let configs: &[&[egl::Int]] = &[
        // Primary: RGBA8 + Depth16 + Stencil8
        &[
            egl::RED_SIZE,
            8,
            egl::GREEN_SIZE,
            8,
            egl::BLUE_SIZE,
            8,
            egl::ALPHA_SIZE,
            8,
            egl::DEPTH_SIZE,
            16,
            egl::STENCIL_SIZE,
            8,
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT | egl::PBUFFER_BIT,
            egl::RENDERABLE_TYPE,
            egl::OPENGL_ES2_BIT,
            egl::NONE,
        ],
        // Fallback 1: Depth24 + Stencil8 (some drivers only offer D24S8 packed)
        &[
            egl::RED_SIZE,
            8,
            egl::GREEN_SIZE,
            8,
            egl::BLUE_SIZE,
            8,
            egl::ALPHA_SIZE,
            8,
            egl::DEPTH_SIZE,
            24,
            egl::STENCIL_SIZE,
            8,
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT | egl::PBUFFER_BIT,
            egl::RENDERABLE_TYPE,
            egl::OPENGL_ES2_BIT,
            egl::NONE,
        ],
        // Fallback 2: no stencil (older/simpler GPUs)
        &[
            egl::RED_SIZE,
            8,
            egl::GREEN_SIZE,
            8,
            egl::BLUE_SIZE,
            8,
            egl::ALPHA_SIZE,
            8,
            egl::DEPTH_SIZE,
            16,
            egl::STENCIL_SIZE,
            0,
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT | egl::PBUFFER_BIT,
            egl::RENDERABLE_TYPE,
            egl::OPENGL_ES2_BIT,
            egl::NONE,
        ],
    ];

    let mut config = None;
    for attrs in configs {
        if let Ok(Some(c)) = egl.choose_first_config(display, attrs) {
            config = Some(c);
            break;
        }
    }
    let config = config.ok_or_else(|| {
        ee(
            ErrorCode::RenderChooseConfigError,
            "all EGL config candidates failed",
        )
    })?;

    Ok(EglInitResult {
        egl,
        display,
        config,
    })
}

/// Create a pbuffer context with optional share context
pub(super) fn create_pbuffer_context(
    egl: &egl::DynamicInstance<EGL1_4>,
    display: egl::Display,
    config: egl::Config,
    share: Option<egl::Context>,
    width: u32,
    height: u32,
) -> EngineResult<(egl::Context, egl::Surface)> {
    let ctx_attribs = [egl::CONTEXT_CLIENT_VERSION as i32, 2, egl::NONE as i32];

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
