extern crate khronos_egl as egl;

use femtovg::{Canvas as FvCanvas, renderer::OpenGl};
use shared::{
    error::{EngineResult, ErrorCode},
    protocol::render_cmd::CanvasId,
};
use std::{ffi::c_void, ptr};

use super::CanvasManager;
use super::types::ee;
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

    // Use physical pixel dimensions for the femtovg backing store.
    // CanvasInfo.width/height stores logical (CSS) pixels, so we must
    // convert back to physical via DPI for set_size().
    let (phys_w, phys_h) = {
        let canvas = cm.canvases.get(&canvas_id).ok_or_else(|| {
            ee(
                ErrorCode::NotFound,
                format!("canvas not found: id={}", canvas_id),
            )
        })?;
        let dpi = cm.dpi.max(1.0);
        (
            (canvas.logical_width as f32 * dpi).round() as u32,
            (canvas.logical_height as f32 * dpi).round() as u32,
        )
    };

    let dpi = cm.dpi;

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

    fv_canvas.set_size(phys_w, phys_h, dpi);
    fv_canvas.scale(dpi, dpi);

    let font_manager = FontManager::new(&mut fv_canvas)?;
    let ctx2d = Canvas2DContext::new(fv_canvas, font_manager);
    cm.contexts_2d.insert(canvas_id, ctx2d);
    Ok(())
}

/// Flush all dirty 2D contexts
pub(super) fn flush_dirty_2d_contexts(cm: &mut CanvasManager) -> EngineResult<()> {
    let saved = cm.bound;

    let dirty_ids: Vec<CanvasId> = cm.dirty_2d.drain().collect();
    for id in dirty_ids {
        if !cm.contexts_2d.contains_key(&id) {
            continue;
        }
        cm.make_current_needed(id)?;
        if let Some(ctx) = cm.contexts_2d.get_mut(&id) {
            ctx.flush();
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
    Ok(())
}
