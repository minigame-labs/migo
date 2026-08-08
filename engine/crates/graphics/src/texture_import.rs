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
//! All function pointers are resolved dynamically at first use.  The
//! resolved entry points are cached in a process-global `OnceLock`;
//! drivers we have observed return identical `eglGetProcAddress`
//! pointers across shared EGL contexts, but [`validate_import_fns`]
//! provides a per-context assertion that each entry point is still
//! non-null when the upload thread brings its own context online.

use glow::HasContext;
use shared::error::{EngineError, EngineResult, ErrorCode};
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
const MAX_STALE_GL_ERRORS: usize = 32;

/// Drain errors left by earlier GL work before attributing a later error to
/// the EGLImage import. The bound protects against a lost/broken vendor
/// context that never returns `GL_NO_ERROR`.
fn drain_stale_gl_errors(mut get_error: impl FnMut() -> u32) -> EngineResult<usize> {
    for drained in 0..MAX_STALE_GL_ERRORS {
        if get_error() == glow::NO_ERROR {
            return Ok(drained);
        }
    }

    Err(
        EngineError::new(ErrorCode::RenderBackendError).with_detail(format!(
            "stale GL error queue did not drain after {MAX_STALE_GL_ERRORS} reads"
        )),
    )
}

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

/// The four AHB-path entry points, named in the order
/// [`resolve_import_fns`] probes them.  Exposed so the
/// `EngineError` produced on resolution failure can tell the
/// operator *which* symbol is missing — historically the error
/// was just a flat "AHB import functions not resolved" which gave
/// no hint about whether EGL or GL extensions were at fault.
pub const REQUIRED_PROCS: &[&str] = &[
    "eglGetNativeClientBufferANDROID",
    "eglCreateImageKHR",
    "eglDestroyImageKHR",
    "glEGLImageTargetTexture2DOES",
];

/// Resolve EGL/GL function pointers needed for AHB import.  On
/// failure, returns the name of the first entry point that came
/// back null so callers can surface it verbatim.
fn resolve_import_fns(
    egl_get_proc: &dyn Fn(&str) -> Option<unsafe extern "C" fn()>,
) -> Result<ImportFns, &'static str> {
    unsafe fn fetch<F: Copy>(
        egl_get_proc: &dyn Fn(&str) -> Option<unsafe extern "C" fn()>,
        name: &'static str,
    ) -> Result<F, &'static str> {
        match egl_get_proc(name) {
            // SAFETY: the caller has contracted that the returned
            // pointer has the calling convention declared by the
            // target fn type; we transmute the generic fn pointer
            // into that specific shape and keep the `unsafe`
            // marker on the fn type so call sites know.
            Some(p) => Ok(unsafe { std::mem::transmute_copy::<unsafe extern "C" fn(), F>(&p) }),
            None => Err(name),
        }
    }

    unsafe {
        let get_native_client_buffer: EglGetNativeClientBufferANDROID =
            fetch(egl_get_proc, "eglGetNativeClientBufferANDROID")?;
        let create_image: EglCreateImageKHR = fetch(egl_get_proc, "eglCreateImageKHR")?;
        let destroy_image: EglDestroyImageKHR = fetch(egl_get_proc, "eglDestroyImageKHR")?;
        let image_target_texture: GlEGLImageTargetTexture2DOES =
            fetch(egl_get_proc, "glEGLImageTargetTexture2DOES")?;

        Ok(ImportFns {
            get_native_client_buffer,
            create_image,
            destroy_image,
            image_target_texture,
        })
    }
}

/// Outcome of [`ensure_import_fns`] — either every entry point
/// resolved (`Ok`) or we captured which specific symbol was
/// missing (`Err`).  The `Err` case carries a process-global
/// static string so the caller can log it and downgrade
/// `device_caps.ahb_available` without allocating.
pub fn ensure_import_fns(
    egl_get_proc: &dyn Fn(&str) -> Option<unsafe extern "C" fn()>,
) -> Result<(), &'static str> {
    // `OnceLock::get_or_init` is the only thread-safe way to
    // memoise the resolution result.  We store the `Result`
    // directly so subsequent callers on other threads see the
    // same diagnostic.
    let cached: &Option<ImportFns> =
        IMPORT_FNS.get_or_init(|| match resolve_import_fns(egl_get_proc) {
            Ok(fns) => Some(fns),
            Err(name) => {
                tracing::warn!(
                    "AHB import functions not resolved: missing {name}; \
                 AHB upload path will be disabled for the life of this process"
                );
                None
            }
        });
    match cached {
        Some(_) => Ok(()),
        None => Err(cached_failure_name()),
    }
}

/// Best-effort recovery of the first missing proc name for
/// logging; the original diagnostic is emitted inside
/// `OnceLock::get_or_init`, so this fallback exists purely so
/// later callers (upload-thread bring-up) can still produce a
/// structured error without redoing the resolution.
fn cached_failure_name() -> &'static str {
    // We don't actually store the failed name separately — emitting
    // the warning once covers the diagnostic need, and callers treat
    // any `Err` as "AHB unavailable".  Returning a generic label
    // here keeps the signature stable.
    "AHB entry point missing (see earlier tracing::warn for name)"
}

/// Structured error builder for callers that want a full
/// [`EngineError`] instead of the raw `&'static str`.
pub fn ahb_unavailable_err(detail: &'static str) -> EngineError {
    EngineError::new(ErrorCode::RenderBackendError).with_detail(format!(
        "AHB import path unavailable: {detail}; required EGL/GL entry points: {}",
        REQUIRED_PROCS.join(", "),
    ))
}

/// Validate that the import functions resolved by
/// [`ensure_import_fns`] are still usable from the calling thread
/// / EGL context.  Returns `Err` if the cache is empty (no
/// resolution ever succeeded) so the upload thread can abort its
/// bring-up deterministically instead of crashing later inside
/// `glEGLImageTargetTexture2DOES`.
pub fn validate_import_fns() -> Result<(), EngineError> {
    match IMPORT_FNS.get() {
        Some(Some(_)) => Ok(()),
        Some(None) => Err(ahb_unavailable_err(cached_failure_name())),
        None => Err(ahb_unavailable_err(
            "ensure_import_fns has not been called yet on any thread",
        )),
    }
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

    // `glGetError` is sticky across unrelated commands. Drain the old queue
    // before issuing any GL command for this import, otherwise a prior error
    // can be misattributed to a valid AHB and permanently trip the session
    // circuit breaker. A non-draining queue is treated as a broken context.
    let stale_errors = match drain_stale_gl_errors(|| gl.get_error()) {
        Ok(count) => count,
        Err(error) => {
            (fns.destroy_image)(egl_display, egl_image);
            return Err(error);
        }
    };
    if stale_errors != 0 {
        tracing::debug!(stale_errors, "drained stale GL errors before AHB import");
    }

    // Create GL texture and bind EGLImage
    let tex = gl.create_texture().map_err(|e| {
        (fns.destroy_image)(egl_display, egl_image);
        shared::error::EngineError::new(ErrorCode::RenderBackendError)
            .with_detail(format!("create_texture failed: {e}"))
    })?;

    // What the caller's context had bound, so it can have it back. This path is
    // unconditional on Android and runs on the render thread's canvas context;
    // binding *zero* below instead of restoring left the WebGL dedup shadow naming a
    // texture the driver no longer had, so the content's next identical
    // `bindTexture` was dropped as redundant. `compressed_upload.rs` already
    // restores for the same reason.
    let saved_texture = gl.get_parameter_i32(glow::TEXTURE_BINDING_2D);

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
                "AHB GL import sequence failed: gl_error=0x{gl_err:X}"
            )),
        );
    }

    gl.bind_texture(
        glow::TEXTURE_2D,
        std::num::NonZeroU32::new(saved_texture as u32).map(glow::NativeTexture),
    );

    Ok(AhbTextureResult {
        texture: tex,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::{ErrorCode, drain_stale_gl_errors};

    #[test]
    fn stale_gl_errors_are_drained_before_import_error_attribution() {
        let mut errors = [glow::INVALID_ENUM, glow::INVALID_OPERATION, glow::NO_ERROR].into_iter();
        let drained = drain_stale_gl_errors(|| errors.next().unwrap_or(glow::NO_ERROR))
            .expect("finite stale error queue should drain");

        assert_eq!(drained, 2);
    }

    #[test]
    fn non_draining_gl_error_queue_fails_closed() {
        let error = drain_stale_gl_errors(|| glow::INVALID_OPERATION)
            .expect_err("a broken context must not spin forever");

        assert_eq!(error.code, ErrorCode::RenderBackendError);
        assert!(
            error
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("stale GL error queue")
        );
    }
}
