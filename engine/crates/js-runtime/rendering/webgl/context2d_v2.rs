//! Canvas 2D Context V2 - Command Batching Implementation
//!
//! This module provides command batching for Canvas 2D operations,
//! collecting all commands within a frame and sending them as a single batch.
//!
//! ## Usage Flow
//!
//! ```text
//! RAF starts:
//!   op_frame_begin()           → Creates/resets CommandBuffer
//!   
//! During frame:
//!   op_fill_rect_v2()          → Adds to CommandBuffer
//!   op_draw_image_v2()         → Adds to CommandBuffer
//!   ...more operations...
//!   
//! RAF ends:
//!   op_frame_end()             → Sends CommandBuffer to render thread
//! ```

use deno_core::{op2, OpState};
use femtovg::Color;
use std::cell::RefCell;
use tracing::{error, trace};

use shared::op_state::CanvasOpState;
use shared::protocol::render_cmd::{Canvas2DCmd, RenderCommand, TextAlign, TextBaseline};

/// Per-canvas frame command buffer
struct FrameBuffer {
    commands: Vec<Canvas2DCmd>,
    /// Shadow state for deduplication
    fill_color: Option<Color>,
    stroke_color: Option<Color>,
    line_width: Option<f32>,
    global_alpha: Option<f32>,
}

impl FrameBuffer {
    fn new() -> Self {
        Self {
            commands: Vec::with_capacity(256),
            fill_color: None,
            stroke_color: None,
            line_width: None,
            global_alpha: None,
        }
    }
    
    fn clear(&mut self) {
        self.commands.clear();
        self.fill_color = None;
        self.stroke_color = None;
        self.line_width = None;
        self.global_alpha = None;
    }
    
    #[inline]
    fn push(&mut self, cmd: Canvas2DCmd) {
        self.commands.push(cmd);
    }
    
    /// Set fill color with deduplication
    fn set_fill_color(&mut self, color: Color) {
        if self.fill_color != Some(color) {
            self.fill_color = Some(color);
            self.commands.push(Canvas2DCmd::SetFillStyle { color });
        }
    }
    
    /// Set stroke color with deduplication
    fn set_stroke_color(&mut self, color: Color) {
        if self.stroke_color != Some(color) {
            self.stroke_color = Some(color);
            self.commands.push(Canvas2DCmd::SetStrokeStyle { color });
        }
    }
    
    /// Set line width with deduplication
    fn set_line_width(&mut self, width: f32) {
        if self.line_width != Some(width) {
            self.line_width = Some(width);
            self.commands.push(Canvas2DCmd::SetLineWidth { width });
        }
    }
    
    /// Set global alpha with deduplication
    fn set_global_alpha(&mut self, alpha: f32) {
        if self.global_alpha != Some(alpha) {
            self.global_alpha = Some(alpha);
            self.commands.push(Canvas2DCmd::SetGlobalAlpha { alpha });
        }
    }
}

/// Frame command collector stored in OpState
///
/// Manages command buffers for multiple canvases within a single frame.
pub struct FrameCommandCollector {
    /// Active frame buffers by canvas ID
    buffers: RefCell<std::collections::HashMap<u32, FrameBuffer>>,
    /// Statistics
    total_commands: u64,
    total_frames: u64,
}

impl FrameCommandCollector {
    pub fn new() -> Self {
        Self {
            buffers: RefCell::new(std::collections::HashMap::new()),
            total_commands: 0,
            total_frames: 0,
        }
    }
    
    /// Get or create a frame buffer for a canvas
    fn get_or_create_buffer(&self, canvas_id: u32) -> std::cell::RefMut<'_, FrameBuffer> {
        let mut buffers = self.buffers.borrow_mut();
        if !buffers.contains_key(&canvas_id) {
            buffers.insert(canvas_id, FrameBuffer::new());
        }
        std::cell::RefMut::map(buffers, |b| b.get_mut(&canvas_id).unwrap())
    }
    
    /// Begin a new frame for a canvas
    pub fn frame_begin(&self, canvas_id: u32) {
        let mut buffers = self.buffers.borrow_mut();
        if let Some(buf) = buffers.get_mut(&canvas_id) {
            buf.clear();
        } else {
            buffers.insert(canvas_id, FrameBuffer::new());
        }
    }
    
    /// End frame and get commands to send
    pub fn frame_end(&mut self, canvas_id: u32) -> Option<Vec<Canvas2DCmd>> {
        let mut buffers = self.buffers.borrow_mut();
        if let Some(buf) = buffers.get_mut(&canvas_id) {
            if buf.commands.is_empty() {
                return None;
            }
            
            self.total_commands += buf.commands.len() as u64;
            self.total_frames += 1;
            
            let commands = std::mem::take(&mut buf.commands);
            buf.clear();
            Some(commands)
        } else {
            None
        }
    }
    
    /// Add a command to the current frame
    #[inline]
    pub fn push(&self, canvas_id: u32, cmd: Canvas2DCmd) {
        self.get_or_create_buffer(canvas_id).push(cmd);
    }
    
    /// Set fill color with deduplication
    #[inline]
    pub fn set_fill_color(&self, canvas_id: u32, color: Color) {
        self.get_or_create_buffer(canvas_id).set_fill_color(color);
    }
    
    /// Set stroke color with deduplication
    #[inline]
    pub fn set_stroke_color(&self, canvas_id: u32, color: Color) {
        self.get_or_create_buffer(canvas_id).set_stroke_color(color);
    }
    
    /// Set line width with deduplication
    #[inline]
    pub fn set_line_width(&self, canvas_id: u32, width: f32) {
        self.get_or_create_buffer(canvas_id).set_line_width(width);
    }
    
    /// Set global alpha with deduplication
    #[inline]
    pub fn set_global_alpha(&self, canvas_id: u32, alpha: f32) {
        self.get_or_create_buffer(canvas_id).set_global_alpha(alpha);
    }
}

impl Default for FrameCommandCollector {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Frame lifecycle operations
// ============================================================================

/// Begin a new frame - call at the start of RAF callback
#[op2(fast)]
pub fn op_frame_begin(state: &mut OpState, #[smi] canvas_id: u32) {
    if let Some(collector) = state.try_borrow_mut::<FrameCommandCollector>() {
        collector.frame_begin(canvas_id);
    }
}

/// End frame and submit to render thread
#[op2(fast)]
pub fn op_frame_end(state: &mut OpState, #[smi] canvas_id: u32) {
    // Get commands from collector
    let commands = {
        if let Some(collector) = state.try_borrow_mut::<FrameCommandCollector>() {
            collector.frame_end(canvas_id)
        } else {
            None
        }
    };
    
    // Send batched commands
    if let Some(cmds) = commands {
        if !cmds.is_empty() {
            let ctx = state.borrow::<CanvasOpState>();
            trace!("op_frame_end: sending {} commands for canvas {}", cmds.len(), canvas_id);
            
            // Send as batch command
            if let Err(e) = ctx.tx.send(RenderCommand::Canvas2DBatch { 
                canvas_id, 
                commands: cmds 
            }) {
                error!("op_frame_end: send failed: {e}");
            }
        }
    }
}

/// End frame for all active canvases - called automatically at end of RAF
#[op2(fast)]
pub fn op_frame_end_all(state: &mut OpState) {
    let canvas_ids: Vec<u32> = {
        if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
            collector.buffers.borrow().keys().copied().collect()
        } else {
            return;
        }
    };

    for canvas_id in canvas_ids {
        let commands = {
            if let Some(collector) = state.try_borrow_mut::<FrameCommandCollector>() {
                collector.frame_end(canvas_id)
            } else {
                None
            }
        };

        if let Some(cmds) = commands {
            if !cmds.is_empty() {
                let ctx = state.borrow::<CanvasOpState>();
                trace!("op_frame_end_all: sending {} commands for canvas {}", cmds.len(), canvas_id);
                if let Err(e) = ctx.tx.send(RenderCommand::Canvas2DBatch {
                    canvas_id,
                    commands: cmds,
                }) {
                    error!("op_frame_end_all: send failed: {e}");
                }
            }
        }
    }
}

/// Invalidate canvas for on-demand rendering mode
#[op2(fast)]
pub fn op_invalidate(state: &mut OpState, #[smi] _canvas_id: u32) {
    // Send invalidation signal to render thread
    let ctx = state.borrow::<CanvasOpState>();
    if let Err(e) = ctx.tx.send(RenderCommand::Invalidate) {
        error!("op_invalidate: send failed: {e}");
    }
}

// ============================================================================
// V2 Canvas 2D operations (batched)
// ============================================================================

// Helper macro for simple commands
macro_rules! batched_op {
    ($fn_name:ident, $cmd:expr) => {
        #[op2(fast)]
        pub fn $fn_name(state: &mut OpState, #[smi] canvas_id: u32) {
            if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
                collector.push(canvas_id, $cmd);
            }
        }
    };
}

// Path operations
batched_op!(op_begin_path_v2, Canvas2DCmd::BeginPath);
batched_op!(op_close_path_v2, Canvas2DCmd::ClosePath);
batched_op!(op_fill_v2, Canvas2DCmd::Fill);
batched_op!(op_stroke_v2, Canvas2DCmd::Stroke);
batched_op!(op_clip_v2, Canvas2DCmd::Clip);

// State operations
batched_op!(op_save_v2, Canvas2DCmd::Save);

#[op2(fast)]
pub fn op_restore_v2(state: &mut OpState, #[smi] canvas_id: u32) {
    if let Some(collector) = state.try_borrow_mut::<FrameCommandCollector>() {
        // Clear shadow state on restore
        let mut buffers = collector.buffers.borrow_mut();
        if let Some(buf) = buffers.get_mut(&canvas_id) {
            buf.fill_color = None;
            buf.stroke_color = None;
            buf.line_width = None;
            buf.global_alpha = None;
            buf.commands.push(Canvas2DCmd::Restore);
        }
    }
}

// Transform operations
batched_op!(op_reset_transform_v2, Canvas2DCmd::ResetTransform);

#[op2(fast)]
pub fn op_move_to_v2(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::MoveTo { x, y });
    }
}

#[op2(fast)]
pub fn op_line_to_v2(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::LineTo { x, y });
    }
}

#[op2(fast)]
pub fn op_quadratic_curve_to_v2(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    cpx: f32, cpy: f32,
    x: f32, y: f32,
) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::QuadraticCurveTo { cpx, cpy, x, y });
    }
}

#[op2(fast)]
pub fn op_bezier_curve_to_v2(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    cp1x: f32, cp1y: f32,
    cp2x: f32, cp2y: f32,
    x: f32, y: f32,
) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::BezierCurveTo { cp1x, cp1y, cp2x, cp2y, x, y });
    }
}

#[op2(fast)]
pub fn op_arc_v2(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    x: f32, y: f32,
    radius: f32,
    start_angle: f32, end_angle: f32,
    counterclockwise: bool,
) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::Arc { 
            x, y, radius, start_angle, end_angle, counterclockwise 
        });
    }
}

#[op2(fast)]
pub fn op_arc_to_v2(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    x1: f32, y1: f32,
    x2: f32, y2: f32,
    radius: f32,
) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::ArcTo { x1, y1, x2, y2, radius });
    }
}

#[op2(fast)]
pub fn op_rect_v2(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32, w: f32, h: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::Rect { x, y, w, h });
    }
}

#[op2(fast)]
#[allow(clippy::too_many_arguments)]
pub fn op_ellipse_v2(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    x: f32, y: f32,
    radius_x: f32, radius_y: f32,
    rotation: f32,
    start_angle: f32, end_angle: f32,
    counterclockwise: bool,
) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::Ellipse { 
            x, y, radius_x, radius_y, rotation, start_angle, end_angle, counterclockwise 
        });
    }
}

// Rectangle operations
#[op2(fast)]
pub fn op_fill_rect_v2(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32, w: f32, h: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::FillRect { x, y, w, h });
    }
}

#[op2(fast)]
pub fn op_stroke_rect_v2(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32, w: f32, h: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::StrokeRect { x, y, w, h });
    }
}

#[op2(fast)]
pub fn op_clear_rect_v2(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32, w: f32, h: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::ClearRect { x, y, w, h });
    }
}

// Text operations
#[op2(fast)]
pub fn op_fill_text_v2(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[string] text: String,
    x: f32, y: f32,
    max_width: f32,
) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::FillText { text, x, y, max_width });
    }
}

#[op2(fast)]
pub fn op_stroke_text_v2(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[string] text: String,
    x: f32, y: f32,
    max_width: f32,
) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::StrokeText { text, x, y, max_width });
    }
}

// Style operations (with deduplication)
#[op2(fast)]
pub fn op_set_fill_style_v2(state: &mut OpState, #[smi] canvas_id: u32, #[string] color_str: String) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        let color = super::context2d::parse_color_string(&color_str);
        collector.set_fill_color(canvas_id, color);
    }
}

#[op2(fast)]
pub fn op_set_stroke_style_v2(state: &mut OpState, #[smi] canvas_id: u32, #[string] color_str: String) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        let color = super::context2d::parse_color_string(&color_str);
        collector.set_stroke_color(canvas_id, color);
    }
}

#[op2(fast)]
pub fn op_set_line_width_v2(state: &mut OpState, #[smi] canvas_id: u32, width: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.set_line_width(canvas_id, width);
    }
}

#[op2(fast)]
pub fn op_set_line_cap_v2(state: &mut OpState, #[smi] canvas_id: u32, #[smi] cap: u8) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::SetLineCap { cap });
    }
}

#[op2(fast)]
pub fn op_set_line_join_v2(state: &mut OpState, #[smi] canvas_id: u32, #[smi] join: u8) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::SetLineJoin { join });
    }
}

#[op2(fast)]
pub fn op_set_miter_limit_v2(state: &mut OpState, #[smi] canvas_id: u32, limit: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::SetMiterLimit { limit });
    }
}

#[op2(fast)]
pub fn op_set_global_alpha_v2(state: &mut OpState, #[smi] canvas_id: u32, alpha: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.set_global_alpha(canvas_id, alpha);
    }
}

#[op2(fast)]
pub fn op_set_font_v2(state: &mut OpState, #[smi] canvas_id: u32, #[string] font: String) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::SetFont { font });
    }
}

#[op2(fast)]
pub fn op_set_text_align_v2(state: &mut OpState, #[smi] canvas_id: u32, #[smi] align: u8) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        let align = match align {
            0 => TextAlign::Start,
            1 => TextAlign::End,
            2 => TextAlign::Left,
            3 => TextAlign::Right,
            4 => TextAlign::Center,
            _ => TextAlign::Start,
        };
        collector.push(canvas_id, Canvas2DCmd::SetTextAlign { align });
    }
}

#[op2(fast)]
pub fn op_set_text_baseline_v2(state: &mut OpState, #[smi] canvas_id: u32, #[smi] baseline: u8) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        let baseline = match baseline {
            0 => TextBaseline::Top,
            1 => TextBaseline::Hanging,
            2 => TextBaseline::Middle,
            3 => TextBaseline::Alphabetic,
            4 => TextBaseline::Ideographic,
            5 => TextBaseline::Bottom,
            _ => TextBaseline::Alphabetic,
        };
        collector.push(canvas_id, Canvas2DCmd::SetTextBaseline { baseline });
    }
}

// Transform operations
#[op2(fast)]
pub fn op_translate_v2(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::Translate { x, y });
    }
}

#[op2(fast)]
pub fn op_rotate_v2(state: &mut OpState, #[smi] canvas_id: u32, angle: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::Rotate { angle });
    }
}

#[op2(fast)]
pub fn op_scale_v2(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::Scale { x, y });
    }
}

#[op2(fast)]
#[allow(clippy::too_many_arguments)]
pub fn op_set_transform_v2(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    a: f32, b: f32, c: f32, d: f32, e: f32, f: f32,
) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::SetTransform { a, b, c, d, e, f });
    }
}

// Image operations
#[op2(fast)]
#[allow(clippy::too_many_arguments)]
pub fn op_draw_image_v2(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] image_id: u32,
    sx: f32, sy: f32, sw: f32, sh: f32,
    dx: f32, dy: f32, dw: f32, dh: f32,
) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::DrawImage { 
            image_id, sx, sy, sw, sh, dx, dy, dw, dh 
        });
    }
}

/// Batch draw images (already optimized in V1, kept for compatibility)
#[op2(fast)]
pub fn op_draw_image_batch_v2(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[buffer] data: &[u8],
) {
    use shared::protocol::render_cmd::DrawImageEntry;
    
    const ENTRY_SIZE: usize = 9 * 4;
    
    if data.len() % ENTRY_SIZE != 0 {
        error!("op_draw_image_batch_v2: invalid buffer size");
        return;
    }
    
    let entry_count = data.len() / ENTRY_SIZE;
    if entry_count == 0 {
        return;
    }
    
    let mut draws = Vec::with_capacity(entry_count);
    
    for i in 0..entry_count {
        let offset = i * ENTRY_SIZE;
        let floats: &[f32] = bytemuck::cast_slice(&data[offset..offset + ENTRY_SIZE]);
        
        draws.push(DrawImageEntry {
            image_id: floats[0] as u32,
            sx: floats[1],
            sy: floats[2],
            sw: floats[3],
            sh: floats[4],
            dx: floats[5],
            dy: floats[6],
            dw: floats[7],
            dh: floats[8],
        });
    }
    
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::DrawImageBatch { draws });
    }
}
