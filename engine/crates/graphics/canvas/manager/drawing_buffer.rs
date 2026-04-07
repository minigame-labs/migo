//! Chromium-style DrawingBuffer: an intermediate FBO that WebGL renders to
//! instead of the native window surface directly.
//!
//! On every frame present the color attachment is blitted to the real window
//! surface via `glBlitFramebuffer` (ES 3.0) before `eglSwapBuffers`.

use glow::HasContext;
use shared::error::{EngineResult, ErrorCode};

use super::types::ee;

/// Intermediate render target for the onscreen canvas.
pub(crate) struct DrawingBuffer {
    /// FBO that WebGL commands target when `bindFramebuffer(null)` is called.
    pub fbo: glow::NativeFramebuffer,
    /// Color attachment (RGBA8 texture).
    pub color_tex: glow::NativeTexture,
    /// Depth + stencil attachment (renderbuffer).
    pub depth_stencil_rb: glow::NativeRenderbuffer,
    /// Current buffer width in physical pixels.
    pub width: u32,
    /// Current buffer height in physical pixels.
    pub height: u32,
}

/// Create a new DrawingBuffer at the given dimensions.
///
/// The caller must ensure an EGL context is current.
pub(crate) fn create(gl: &glow::Context, width: u32, height: u32) -> EngineResult<DrawingBuffer> {
    unsafe {
        // Save current bindings to restore later.
        let prev_tex = gl.get_parameter_i32(glow::TEXTURE_BINDING_2D) as u32;
        let prev_rb = gl.get_parameter_i32(glow::RENDERBUFFER_BINDING) as u32;
        let prev_fbo = gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING) as u32;

        let fbo = gl.create_framebuffer().map_err(|e| {
            ee(
                ErrorCode::RenderBackendError,
                format!("DrawingBuffer: create_framebuffer failed: {e}"),
            )
        })?;
        let color_tex = gl.create_texture().map_err(|e| {
            gl.delete_framebuffer(fbo);
            ee(
                ErrorCode::RenderBackendError,
                format!("DrawingBuffer: create_texture failed: {e}"),
            )
        })?;
        let depth_stencil_rb = gl.create_renderbuffer().map_err(|e| {
            gl.delete_framebuffer(fbo);
            gl.delete_texture(color_tex);
            ee(
                ErrorCode::RenderBackendError,
                format!("DrawingBuffer: create_renderbuffer failed: {e}"),
            )
        })?;

        // Allocate color texture.
        gl.bind_texture(glow::TEXTURE_2D, Some(color_tex));
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::NEAREST as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::NEAREST as i32,
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
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            width as i32,
            height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );

        // Allocate depth+stencil renderbuffer.
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth_stencil_rb));
        gl.renderbuffer_storage(
            glow::RENDERBUFFER,
            glow::DEPTH24_STENCIL8,
            width as i32,
            height as i32,
        );

        // Assemble FBO.
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(color_tex),
            0,
        );
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::DEPTH_STENCIL_ATTACHMENT,
            glow::RENDERBUFFER,
            Some(depth_stencil_rb),
        );

        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        if status != glow::FRAMEBUFFER_COMPLETE {
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.delete_framebuffer(fbo);
            gl.delete_texture(color_tex);
            gl.delete_renderbuffer(depth_stencil_rb);
            return Err(ee(
                ErrorCode::RenderBackendError,
                format!("DrawingBuffer: framebuffer incomplete (status=0x{status:X})"),
            ));
        }

        // Ensure framebuffer blit path is usable on this context/driver.
        // `glow::blit_framebuffer` panics if the symbol is not loaded; on
        // some GLES2 stacks this can happen. We probe once here and fall back
        // to direct-to-surface rendering if unavailable.
        clear_gl_errors(gl);
        let blit_probe = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(fbo));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
            gl.blit_framebuffer(
                0,
                0,
                1,
                1,
                0,
                0,
                1,
                1,
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
        }));
        let blit_err = gl.get_error();
        if blit_probe.is_err() || blit_err != glow::NO_ERROR {
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.delete_framebuffer(fbo);
            gl.delete_texture(color_tex);
            gl.delete_renderbuffer(depth_stencil_rb);
            return Err(ee(
                ErrorCode::RenderBackendError,
                if blit_probe.is_err() {
                    "DrawingBuffer: glBlitFramebuffer not available in this GL context".to_string()
                } else {
                    format!(
                        "DrawingBuffer: glBlitFramebuffer probe failed (gl_error=0x{blit_err:X})"
                    )
                },
            ));
        }
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));

        // Restore previous bindings.
        restore_texture_binding(gl, prev_tex);
        restore_renderbuffer_binding(gl, prev_rb);
        // Leave the DrawingBuffer FBO bound — caller expects it for onscreen canvas.
        if prev_fbo != 0 {
            // Only restore if there was a non-default FBO (shouldn't happen at init).
            // We intentionally keep our FBO bound as the "default" for onscreen.
        }

        Ok(DrawingBuffer {
            fbo,
            color_tex,
            depth_stencil_rb,
            width,
            height,
        })
    }
}

/// Resize the DrawingBuffer storage without recreating GL objects.
pub(crate) fn resize(
    gl: &glow::Context,
    db: &mut DrawingBuffer,
    new_w: u32,
    new_h: u32,
) -> EngineResult<()> {
    if db.width == new_w && db.height == new_h {
        return Ok(());
    }

    unsafe {
        let prev_tex = gl.get_parameter_i32(glow::TEXTURE_BINDING_2D) as u32;
        let prev_rb = gl.get_parameter_i32(glow::RENDERBUFFER_BINDING) as u32;

        // Re-allocate color texture.
        gl.bind_texture(glow::TEXTURE_2D, Some(db.color_tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            new_w as i32,
            new_h as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );

        // Re-allocate depth+stencil renderbuffer.
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(db.depth_stencil_rb));
        gl.renderbuffer_storage(
            glow::RENDERBUFFER,
            glow::DEPTH24_STENCIL8,
            new_w as i32,
            new_h as i32,
        );

        // Re-attach (required on some drivers after storage reallocation).
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(db.fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(db.color_tex),
            0,
        );
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::DEPTH_STENCIL_ATTACHMENT,
            glow::RENDERBUFFER,
            Some(db.depth_stencil_rb),
        );

        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        if status != glow::FRAMEBUFFER_COMPLETE {
            return Err(ee(
                ErrorCode::RenderBackendError,
                format!("DrawingBuffer resize: framebuffer incomplete (status=0x{status:X})"),
            ));
        }

        // Restore texture/renderbuffer bindings.
        restore_texture_binding(gl, prev_tex);
        restore_renderbuffer_binding(gl, prev_rb);
        // Leave DrawingBuffer FBO bound.
    }

    db.width = new_w;
    db.height = new_h;
    Ok(())
}

/// Destroy the DrawingBuffer and release all GL resources.
pub(crate) fn destroy(gl: &glow::Context, db: DrawingBuffer) {
    unsafe {
        gl.delete_framebuffer(db.fbo);
        gl.delete_texture(db.color_tex);
        gl.delete_renderbuffer(db.depth_stencil_rb);
    }
}

/// Blit the DrawingBuffer color attachment to the real default framebuffer (FBO 0).
///
/// Uses `glBlitFramebuffer` (ES 3.0). `surface_w`/`surface_h` are the actual
/// EGL window surface dimensions (the blit destination).
///
/// If the DrawingBuffer FBO has become incomplete (e.g. the game's WebGL code
/// modified its attachments), this function re-attaches the original textures
/// before retrying the blit.
#[inline]
pub(crate) fn blit_to_surface(
    gl: &glow::Context,
    db: &DrawingBuffer,
    surface_w: u32,
    surface_h: u32,
) {
    unsafe {
        // Clear any pending GL error.
        clear_gl_errors(gl);

        // READ from DrawingBuffer, DRAW to window surface (FBO 0).
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(db.fbo));
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);

        let err = gl.get_error();
        if err != glow::NO_ERROR {
            tracing::warn!(
                "DrawingBuffer blit: bind failed (gl_error=0x{err:X}), db={}x{} surface={}x{}",
                db.width,
                db.height,
                surface_w,
                surface_h
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            return;
        }

        // Check READ framebuffer completeness before blit.  Game WebGL code
        // can accidentally modify the DrawingBuffer FBO attachments (e.g.
        // framebufferTexture2D on "null" framebuffer), making it incomplete.
        let status = gl.check_framebuffer_status(glow::READ_FRAMEBUFFER);
        if status != glow::FRAMEBUFFER_COMPLETE {
            // Try to heal: re-attach original color + depth/stencil.
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(db.fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(db.color_tex),
                0,
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_STENCIL_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(db.depth_stencil_rb),
            );
            let healed = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if healed != glow::FRAMEBUFFER_COMPLETE {
                tracing::warn!(
                    "DrawingBuffer blit: FBO incomplete (0x{status:X}), re-attach failed (0x{healed:X})"
                );
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                return;
            }
            tracing::debug!("DrawingBuffer blit: FBO healed after re-attach");
            // Re-bind for blit.
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(db.fbo));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
        }

        gl.blit_framebuffer(
            0,
            0,
            db.width as i32,
            db.height as i32,
            0,
            0,
            surface_w as i32,
            surface_h as i32,
            glow::COLOR_BUFFER_BIT,
            glow::LINEAR,
        );

        let err = gl.get_error();
        if err != glow::NO_ERROR {
            tracing::warn!(
                "DrawingBuffer blit: glBlitFramebuffer failed (gl_error=0x{err:X}), db={}x{} surface={}x{}",
                db.width, db.height, surface_w, surface_h
            );
        }
    }
}

/// Restore a texture binding from a raw GL integer (0 = unbind).
unsafe fn restore_texture_binding(gl: &glow::Context, prev: u32) {
    unsafe {
        let handle = if prev == 0 {
            None
        } else {
            Some(glow::NativeTexture(std::num::NonZeroU32::new_unchecked(
                prev,
            )))
        };
        gl.bind_texture(glow::TEXTURE_2D, handle);
    }
}

/// Restore a renderbuffer binding from a raw GL integer (0 = unbind).
unsafe fn restore_renderbuffer_binding(gl: &glow::Context, prev: u32) {
    unsafe {
        let handle = if prev == 0 {
            None
        } else {
            Some(glow::NativeRenderbuffer(
                std::num::NonZeroU32::new_unchecked(prev),
            ))
        };
        gl.bind_renderbuffer(glow::RENDERBUFFER, handle);
    }
}

/// Drain pending GL errors without risking an infinite loop on broken drivers.
#[inline]
unsafe fn clear_gl_errors(gl: &glow::Context) {
    for _ in 0..16 {
        if unsafe { gl.get_error() } == glow::NO_ERROR {
            break;
        }
    }
}
