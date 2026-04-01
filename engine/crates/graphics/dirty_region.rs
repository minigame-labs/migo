//! Dirty region tracking for scissored rendering.
//!
//! When the dirty area of a frame is significantly smaller than the full canvas,
//! enabling GL scissor test avoids redundant fragment processing.

use glow::HasContext;

/// Axis-aligned bounding box of the dirty area in pixel coordinates.
pub struct DirtyRegion {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Apply GL scissor test if the dirty region is less than 50% of the canvas area.
///
/// This avoids the overhead of scissor setup when the dirty region covers most
/// of the canvas (where the benefit would be negligible).
pub fn apply_scissor(gl: &glow::Context, dirty: &DirtyRegion, canvas_w: i32, canvas_h: i32) {
    let dirty_area = (dirty.width as i64) * (dirty.height as i64);
    let canvas_area = (canvas_w as i64) * (canvas_h as i64);
    if canvas_area > 0 && dirty_area < canvas_area / 2 {
        unsafe {
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(dirty.x, dirty.y, dirty.width, dirty.height);
        }
    }
}

/// Disable scissor test, restoring full-canvas rendering.
pub fn clear_scissor(gl: &glow::Context) {
    unsafe {
        gl.disable(glow::SCISSOR_TEST);
    }
}

/// Hint to the driver that the framebuffer regions outside the dirty area
/// need not be preserved. On tiled-GPU architectures (Adreno, Mali, PowerVR)
/// this avoids loading unchanged tiles from main memory into tile SRAM,
/// saving both bandwidth and power.
///
/// Emits up to 4 rectangular invalidation strips (top, bottom, left, right)
/// around the dirty region. Only the clean area is invalidated — the dirty
/// region itself is preserved for upcoming draws.
///
/// Requires OpenGL ES 3.0 (`glInvalidateSubFramebuffer`).
pub fn invalidate_outside_dirty(
    gl: &glow::Context,
    dirty: &DirtyRegion,
    canvas_w: i32,
    canvas_h: i32,
) {
    let dirty_area = (dirty.width as i64) * (dirty.height as i64);
    let canvas_area = (canvas_w as i64) * (canvas_h as i64);
    if canvas_area == 0 || dirty_area >= canvas_area / 2 {
        return;
    }

    let attachments = &[glow::COLOR_ATTACHMENT0];
    let dx = dirty.x.max(0);
    let dy = dirty.y.max(0);
    let dx2 = (dirty.x + dirty.width).min(canvas_w);
    let dy2 = (dirty.y + dirty.height).min(canvas_h);

    unsafe {
        // Top strip (above dirty region)
        if dy > 0 {
            gl.invalidate_sub_framebuffer(
                glow::FRAMEBUFFER, attachments, 0, dy2, canvas_w, canvas_h - dy2,
            );
        }
        // Bottom strip (below dirty region)
        if dy2 < canvas_h {
            gl.invalidate_sub_framebuffer(
                glow::FRAMEBUFFER, attachments, 0, 0, canvas_w, dy,
            );
        }
        // Left strip (left of dirty region, between top and bottom)
        if dx > 0 {
            gl.invalidate_sub_framebuffer(
                glow::FRAMEBUFFER, attachments, 0, dy, dx, dirty.height,
            );
        }
        // Right strip (right of dirty region, between top and bottom)
        if dx2 < canvas_w {
            gl.invalidate_sub_framebuffer(
                glow::FRAMEBUFFER, attachments, dx2, dy, canvas_w - dx2, dirty.height,
            );
        }
    }
}
