//! AHardwareBuffer → EGLImage → GL Texture import path.
//!
//! Bypasses `glTexImage2D` entirely: the decoded RGBA pixels living in the
//! AHardwareBuffer are imported as an EGLImage and bound directly to a GL
//! texture.  This eliminates the CPU→GPU upload copy.
//!
//! Requires:
//! - EGL_ANDROID_image_native_buffer (EGL extension)
//! - GL_OES_EGL_image (GL extension)
//! - Android API 26+ (AHardwareBuffer)
//!
//! All function pointers are resolved dynamically at first use.

use glow::HasContext;
use shared::error::{EngineResult, ErrorCode};
use std::ffi::c_void;
use std::sync::OnceLock;

/// Result of an AHB texture import.
pub struct AhbTextureResult {
    pub texture: glow::NativeTexture,
    pub width: u32,
    pub height: u32,
}

// --- EGL constants not in khronos-egl ---

const EGL_NATIVE_BUFFER_ANDROID: u32 = 0x3140;
const EGL_NO_CONTEXT: *const c_void = std::ptr::null();
const EGL_NONE: i32 = 0x3038;
const EGL_IMAGE_PRESERVED_KHR: i32 = 0x30D2;
const EGL_TRUE: i32 = 1;

// --- Function pointer types ---

type EglGetNativeClientBufferANDROID = unsafe extern "C" fn(buffer: *const c_void) -> *const c_void;
type EglCreateImageKHR = unsafe extern "C" fn(
    dpy: *const c_void,
    ctx: *const c_void,
    target: u32,
    buffer: *const c_void,
    attrib_list: *const i32,
) -> *const c_void;
type EglDestroyImageKHR = unsafe extern "C" fn(dpy: *const c_void, image: *const c_void) -> u32;
type GlEGLImageTargetTexture2DOES = unsafe extern "C" fn(target: u32, image: *const c_void);

struct ImportFns {
    get_native_client_buffer: EglGetNativeClientBufferANDROID,
    create_image: EglCreateImageKHR,
    destroy_image: EglDestroyImageKHR,
    image_target_texture: GlEGLImageTargetTexture2DOES,
}

static IMPORT_FNS: OnceLock<Option<ImportFns>> = OnceLock::new();

/// Resolve EGL/GL function pointers needed for AHB import.
/// Returns None if any required function is missing.
fn resolve_import_fns(
    egl_get_proc: &dyn Fn(&str) -> Option<unsafe extern "C" fn()>,
) -> Option<ImportFns> {
    unsafe {
        let get_native_client_buffer: EglGetNativeClientBufferANDROID =
            std::mem::transmute(egl_get_proc("eglGetNativeClientBufferANDROID")?);
        let create_image: EglCreateImageKHR =
            std::mem::transmute(egl_get_proc("eglCreateImageKHR")?);
        let destroy_image: EglDestroyImageKHR =
            std::mem::transmute(egl_get_proc("eglDestroyImageKHR")?);
        let image_target_texture: GlEGLImageTargetTexture2DOES =
            std::mem::transmute(egl_get_proc("glEGLImageTargetTexture2DOES")?);

        Some(ImportFns {
            get_native_client_buffer,
            create_image,
            destroy_image,
            image_target_texture,
        })
    }
}

/// Lazily resolve and cache import function pointers.
pub fn ensure_import_fns(egl_get_proc: &dyn Fn(&str) -> Option<unsafe extern "C" fn()>) -> bool {
    IMPORT_FNS
        .get_or_init(|| resolve_import_fns(egl_get_proc))
        .is_some()
}

/// Import an AHardwareBuffer as a GL texture.
///
/// `ahb_ptr` is the raw `AHardwareBuffer*` pointer.
/// `egl_display` is the raw `EGLDisplay` pointer.
///
/// Returns a new GL texture with the AHB contents bound.  The AHB can be
/// released after this call — the EGLImage holds a reference.
///
/// # Safety
/// - `ahb_ptr` must be a valid AHardwareBuffer pointer.
/// - `egl_display` must be the current EGL display.
/// - A GL context must be current on the calling thread.
#[allow(unsafe_op_in_unsafe_fn)]
pub unsafe fn import_ahb_as_texture(
    gl: &glow::Context,
    ahb_ptr: *const c_void,
    egl_display: *const c_void,
    width: u32,
    height: u32,
) -> EngineResult<AhbTextureResult> {
    let fns = IMPORT_FNS.get().and_then(|o| o.as_ref()).ok_or_else(|| {
        shared::error::EngineError::new(ErrorCode::RenderBackendError)
            .with_detail("AHB import functions not resolved".to_string())
    })?;

    // AHardwareBuffer → EGLClientBuffer
    let client_buffer = (fns.get_native_client_buffer)(ahb_ptr);
    if client_buffer.is_null() {
        return Err(
            shared::error::EngineError::new(ErrorCode::RenderBackendError)
                .with_detail("eglGetNativeClientBufferANDROID returned null".to_string()),
        );
    }

    // EGLClientBuffer → EGLImage
    let attrs: [i32; 3] = [EGL_IMAGE_PRESERVED_KHR, EGL_TRUE, EGL_NONE];
    let egl_image = (fns.create_image)(
        egl_display,
        EGL_NO_CONTEXT,
        EGL_NATIVE_BUFFER_ANDROID,
        client_buffer,
        attrs.as_ptr(),
    );
    if egl_image.is_null() {
        return Err(
            shared::error::EngineError::new(ErrorCode::RenderBackendError)
                .with_detail("eglCreateImageKHR failed for AHB".to_string()),
        );
    }

    // Create GL texture and bind EGLImage
    let tex = gl.create_texture().map_err(|e| {
        (fns.destroy_image)(egl_display, egl_image);
        shared::error::EngineError::new(ErrorCode::RenderBackendError)
            .with_detail(format!("create_texture failed: {e}"))
    })?;

    gl.bind_texture(glow::TEXTURE_2D, Some(tex));

    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_MIN_FILTER,
        glow::LINEAR as i32,
    );
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_MAG_FILTER,
        glow::LINEAR as i32,
    );
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_WRAP_S,
        glow::CLAMP_TO_EDGE as i32,
    );
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_WRAP_T,
        glow::CLAMP_TO_EDGE as i32,
    );

    // EGLImage → GL_TEXTURE_2D
    (fns.image_target_texture)(glow::TEXTURE_2D, egl_image);

    let gl_err = gl.get_error();
    // EGLImage can be destroyed immediately — the texture holds a ref.
    (fns.destroy_image)(egl_display, egl_image);

    if gl_err != glow::NO_ERROR {
        gl.delete_texture(tex);
        return Err(
            shared::error::EngineError::new(ErrorCode::RenderBackendError).with_detail(format!(
                "glEGLImageTargetTexture2DOES failed: gl_error=0x{gl_err:X}"
            )),
        );
    }

    gl.bind_texture(glow::TEXTURE_2D, None);

    Ok(AhbTextureResult {
        texture: tex,
        width,
        height,
    })
}
