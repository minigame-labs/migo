extern crate khronos_egl as egl;

use femtovg::{renderer::OpenGl, Canvas as FvCanvas};
use glow::HasContext;
use shared::{
    error::{EngineResult, ErrorCode},
    protocol::render_cmd::CanvasId,
};
use std::{ffi::c_void, ptr};

use super::types::ee;
use super::CanvasManager;
use crate::{BoundContext, Canvas2DContext, FontManager};

/// Initialize femtovg for a canvas
pub(super) fn init_femtovg_for_canvas(
    cm: &mut CanvasManager,
    canvas_id: CanvasId,
) -> EngineResult<()> {
    cm.make_current_needed(canvas_id)?;

    if cm.contexts_2d.contains_key(&canvas_id) {
        return Ok(());
    }

    // Canvas dimensions are in physical (buffer) pixels — no DPR conversion needed.
    let (phys_w, phys_h) = {
        let canvas = cm.canvases.get(&canvas_id).ok_or_else(|| {
            ee(
                ErrorCode::NotFound,
                format!("canvas not found: id={}", canvas_id),
            )
        })?;
        (canvas.physical_width, canvas.physical_height)
    };

    let get_proc = |s: &str| -> *const c_void {
        cm.egl
            .get_proc_address(s)
            .map(|f| f as *const c_void)
            .unwrap_or(ptr::null())
    };

    let renderer = unsafe { OpenGl::new_from_function(|s| get_proc(s)) }.map_err(|e| {
        ee(
            ErrorCode::RenderBackendError,
            format!("OpenGl::new_from_function failed: {e:?}"),
        )
    })?;

    let mut fv_canvas = FvCanvas::new(renderer).map_err(|e| {
        ee(
            ErrorCode::RenderBackendError,
            format!("FvCanvas::new failed: {e:?}"),
        )
    })?;

    // dpi = 1.0: Canvas2D coordinates are in buffer pixels (no DPR scaling),
    // matching browser semantics where ctx.fillRect(0,0,100,100) fills
    // exactly 100x100 pixels of the canvas buffer.
    fv_canvas.set_size(phys_w, phys_h, 1.0);

    let font_manager = FontManager::new(&mut fv_canvas)?;
    let ctx2d = Canvas2DContext::new(fv_canvas, font_manager);
    cm.contexts_2d.insert(canvas_id, ctx2d);
    Ok(())
}

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

/// Save current GL state and set a safe baseline for femtovg text atlas upload.
///
/// femtovg's glyph atlas upload (`GlTexture::update`) calls `bind_texture` and
/// `tex_sub_image_2d` without resetting active texture unit or
/// PIXEL_UNPACK_BUFFER binding. If WebGL left those states dirty, uploads may
/// read from wrong source or bind to a wrong unit, causing garbled text.
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

/// Flush all dirty 2D contexts
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
            ctx.flush();
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
