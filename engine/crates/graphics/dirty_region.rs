//! Dirty region tracking for scissored rendering.
//!
//! When the dirty area of a frame is significantly smaller than the full canvas,
//! enabling GL scissor test avoids redundant fragment processing.

use glow::HasContext;

#[path = "damage_tracker.rs"]
pub mod damage_tracker;

/// Axis-aligned bounding box of the dirty area in pixel coordinates.
pub struct DirtyRegion {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl DirtyRegion {
    pub fn as_rect(&self) -> (i32, i32, i32, i32) {
        (self.x, self.y, self.width, self.height)
    }
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

// invalidate_outside_dirty() was removed — it issued glInvalidateSubFramebuffer
// on the DrawingBuffer FBO, but the subsequent full-surface blit_to_surface()
// would read the invalidated (now-undefined) regions, risking garbage pixels
// on tiled GPUs (Mali, PowerVR). Can be reintroduced if the blit path is
// later changed to partial-region only.
