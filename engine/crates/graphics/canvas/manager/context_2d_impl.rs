//! Per-canvas Skia Canvas2D initialisation + flush helpers.
//!
//! This module used to wire up femtovg; it now wraps the
//! `backend::gl::surface::Canvas2DContext` that owns `SkSurface` +
//! `GrDirectContext` for each live 2D canvas.
//!
//! GL-state hygiene helpers live here too (the `begin_canvas2d_gl_scope`
//! guard that Skia's text-atlas upload and WebGL share an EGL context
//! with mutually-visible state — see the struct doc for the full list).

extern crate khronos_egl as egl;

use glow::HasContext;
use shared::{
    error::{EngineResult, ErrorCode},
    protocol::render_cmd::CanvasId,
};

use super::types::ee;
use super::CanvasManager;
use crate::backend::gl::surface::{Canvas2DContext, FboKind};
use crate::BoundContext;

/// Initialise a Skia-backed Canvas2D context for `canvas_id`.
///
/// Idempotent — returns immediately if the context already exists.
/// Caller is responsible for making the target EGL context current
/// before invocation (which this function then double-checks via
/// `make_current_needed`).
pub(super) fn init_skia_for_canvas(
    cm: &mut CanvasManager,
    canvas_id: CanvasId,
) -> EngineResult<()> {
    cm.make_current_needed(canvas_id)?;

    if cm.contexts_2d.contains_key(&canvas_id) {
        return Ok(());
    }

    // Pull the FBO + size from the canvas entry.  Onscreen canvases
    // (id=1) render into the DrawingBuffer FBO; offscreen pbuffers
    // render into the default framebuffer (FBO 0).
    let (fbo_id, width, height, kind) = {
        let entry = cm.canvases.get(&canvas_id).ok_or_else(|| {
            ee(
                ErrorCode::NotFound,
                format!("canvas not found: id={canvas_id}"),
            )
        })?;
        match &entry.drawing_buffer {
            Some(db) => (
                db.fbo.0.get(),
                db.width,
                db.height,
                FboKind::DrawingBuffer,
            ),
            None => (0, entry.physical_width, entry.physical_height, FboKind::DefaultFb),
        }
    };

    let ctx = Canvas2DContext::new(fbo_id, width, height, kind).ok_or_else(|| {
        ee(
            ErrorCode::RenderBackendError,
            format!("Skia Canvas2DContext::new failed for canvas_id={canvas_id} ({width}x{height} fbo={fbo_id})"),
        )
    })?;
    cm.contexts_2d.insert(canvas_id, ctx);
    Ok(())
}

/// GL-state snapshot used to keep Skia's text-atlas upload from tripping
/// WebGL state (or vice versa).
///
/// Skia's glyph rasterisation and `glTexSubImage2D` uploads bind the
/// 0th texture unit and `GL_PIXEL_UNPACK_BUFFER = 0`, and assume the
/// default alignment of 4.  WebGL games mutate these freely, so we
/// snapshot on scope entry and restore on drop.
pub(crate) struct Canvas2DGlState {
    active_texture: i32,
    unpack_pbo: Option<<glow::Context as glow::HasContext>::Buffer>,
    pack_pbo: Option<<glow::Context as glow::HasContext>::Buffer>,
    unpack_alignment: i32,
    pack_alignment: i32,
}

pub(crate) struct Canvas2DGlScopeGuard {
    gl: *const glow::Context,
    state: Option<Canvas2DGlState>,
}

impl Drop for Canvas2DGlScopeGuard {
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let gl = unsafe { &*self.gl };
            unsafe {
                gl.active_texture(state.active_texture as u32);
                gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, state.unpack_pbo);
                gl.bind_buffer(glow::PIXEL_PACK_BUFFER, state.pack_pbo);
                gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, state.unpack_alignment);
                gl.pixel_store_i32(glow::PACK_ALIGNMENT, state.pack_alignment);
            }
        }
    }
}

/// Save + reset GL state for a Skia text-atlas / image-upload sequence.
/// Returns a guard that restores the WebGL-visible state on drop.
pub(super) fn begin_canvas2d_gl_scope(gl: &glow::Context) -> Canvas2DGlScopeGuard {
    unsafe {
        let active_texture = gl.get_parameter_i32(glow::ACTIVE_TEXTURE);
        let unpack_pbo = gl.get_parameter_buffer(glow::PIXEL_UNPACK_BUFFER_BINDING);
        let pack_pbo = gl.get_parameter_buffer(glow::PIXEL_PACK_BUFFER_BINDING);
        let unpack_alignment = gl.get_parameter_i32(glow::UNPACK_ALIGNMENT);
        let pack_alignment = gl.get_parameter_i32(glow::PACK_ALIGNMENT);

        gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
        gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
        gl.pixel_store_i32(glow::PACK_ALIGNMENT, 4);
        gl.active_texture(glow::TEXTURE0);

        Canvas2DGlScopeGuard {
            gl: gl as *const glow::Context,
            state: Some(Canvas2DGlState {
                active_texture,
                unpack_pbo,
                pack_pbo,
                unpack_alignment,
                pack_alignment,
            }),
        }
    }
}

/// Flush all Canvas2D contexts with pending Skia draws.
///
/// Called at frame-end before `eglSwapBuffers` and also at each
/// Canvas2D → WebGL boundary (`Materialize` op).  Returns the list of
/// canvas IDs that actually flushed, so the render-thread layer can
/// clear its per-layer dirty bit.
pub(super) fn flush_dirty_2d_contexts(cm: &mut CanvasManager) -> EngineResult<Vec<CanvasId>> {
    let saved = cm.bound;

    let dirty_ids: Vec<CanvasId> = cm.dirty_2d.drain().collect();
    let mut flushed_ids = Vec::with_capacity(dirty_ids.len());
    for id in dirty_ids {
        if !cm.contexts_2d.contains_key(&id) {
            continue;
        }
        cm.make_current_needed(id)?;
        let _gl_scope = begin_canvas2d_gl_scope(&cm.gl);
        if let Some(ctx) = cm.contexts_2d.get_mut(&id) {
            ctx.flush_and_submit();
            // Drop Skia's internal GL-state tracking so subsequent WebGL
            // / DrawingBuffer-blit code doesn't see stale assumptions.
            ctx.reset_gl_state();
            flushed_ids.push(id);
        }
    }

    match saved {
        BoundContext::Resource => cm.bind_resource()?,
        BoundContext::Canvas(id) => {
            if cm.canvases.contains_key(&id) {
                cm.make_current_needed(id)?;
            } else {
                cm.bind_resource()?;
            }
        }
    }
    Ok(flushed_ids)
}
