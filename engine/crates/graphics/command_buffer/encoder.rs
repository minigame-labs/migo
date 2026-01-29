//! Command encoder for building command buffers
//!
//! The CommandEncoder provides a fluent API for recording rendering commands.
//! It tracks state changes to eliminate redundant commands.

use femtovg::Color;
use super::{Canvas2DCommand, CommandBuffer, DirtyRect, DrawImageParams};

/// Command encoder with state tracking for redundant command elimination
///
/// This encoder maintains a shadow of the current drawing state and
/// only emits commands when the state actually changes.
pub struct CommandEncoder {
    /// The command buffer being built
    buffer: CommandBuffer,
    
    /// Shadow state for deduplication
    state: EncoderState,
    
    /// Whether we're in a path building sequence
    in_path: bool,
}

/// Shadow state for tracking current values
#[derive(Clone)]
struct EncoderState {
    fill_color: Option<Color>,
    stroke_color: Option<Color>,
    line_width: Option<f32>,
    line_cap: Option<u8>,
    line_join: Option<u8>,
    miter_limit: Option<f32>,
    global_alpha: Option<f32>,
    font_family: Option<String>,
    font_size: Option<f32>,
    text_align: Option<u8>,
    text_baseline: Option<u8>,
}

impl Default for EncoderState {
    fn default() -> Self {
        Self {
            fill_color: None,
            stroke_color: None,
            line_width: None,
            line_cap: None,
            line_join: None,
            miter_limit: None,
            global_alpha: None,
            font_family: None,
            font_size: None,
            text_align: None,
            text_baseline: None,
        }
    }
}

impl CommandEncoder {
    /// Create a new encoder for the given canvas
    pub fn new(canvas_id: u32) -> Self {
        Self {
            buffer: CommandBuffer::new(canvas_id),
            state: EncoderState::default(),
            in_path: false,
        }
    }
    
    /// Create with pre-allocated capacity
    pub fn with_capacity(canvas_id: u32, capacity: usize) -> Self {
        Self {
            buffer: CommandBuffer::with_capacity(canvas_id, capacity),
            state: EncoderState::default(),
            in_path: false,
        }
    }
    
    /// Finish encoding and return the command buffer
    pub fn finish(self) -> CommandBuffer {
        self.buffer
    }
    
    /// Get the current command count
    pub fn command_count(&self) -> usize {
        self.buffer.command_count()
    }
    
    // ========== State Commands ==========
    
    /// Save the current state
    pub fn save(&mut self) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::Save);
        self
    }
    
    /// Restore the previous state
    pub fn restore(&mut self) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::Restore);
        // Clear shadow state since restored state is unknown
        self.state = EncoderState::default();
        self
    }
    
    // ========== Style Commands ==========
    
    /// Set fill color (with deduplication)
    pub fn set_fill_color(&mut self, color: Color) -> &mut Self {
        if self.state.fill_color != Some(color) {
            self.state.fill_color = Some(color);
            self.buffer.push_2d(Canvas2DCommand::SetFillColor(color));
        }
        self
    }
    
    /// Set stroke color (with deduplication)
    pub fn set_stroke_color(&mut self, color: Color) -> &mut Self {
        if self.state.stroke_color != Some(color) {
            self.state.stroke_color = Some(color);
            self.buffer.push_2d(Canvas2DCommand::SetStrokeColor(color));
        }
        self
    }
    
    /// Set fill to linear gradient
    pub fn set_fill_gradient(
        &mut self,
        x0: f32, y0: f32,
        x1: f32, y1: f32,
        start: Color, end: Color,
    ) -> &mut Self {
        self.state.fill_color = None; // Gradient, not solid color
        self.buffer.push_2d(Canvas2DCommand::SetFillGradient {
            x0, y0, x1, y1, start, end,
        });
        self
    }
    
    /// Set line width (with deduplication)
    pub fn set_line_width(&mut self, width: f32) -> &mut Self {
        if self.state.line_width != Some(width) {
            self.state.line_width = Some(width);
            self.buffer.push_2d(Canvas2DCommand::SetLineWidth(width));
        }
        self
    }
    
    /// Set line cap
    pub fn set_line_cap(&mut self, cap: u8) -> &mut Self {
        if self.state.line_cap != Some(cap) {
            self.state.line_cap = Some(cap);
            self.buffer.push_2d(Canvas2DCommand::SetLineCap(cap));
        }
        self
    }
    
    /// Set line join
    pub fn set_line_join(&mut self, join: u8) -> &mut Self {
        if self.state.line_join != Some(join) {
            self.state.line_join = Some(join);
            self.buffer.push_2d(Canvas2DCommand::SetLineJoin(join));
        }
        self
    }
    
    /// Set miter limit
    pub fn set_miter_limit(&mut self, limit: f32) -> &mut Self {
        if self.state.miter_limit != Some(limit) {
            self.state.miter_limit = Some(limit);
            self.buffer.push_2d(Canvas2DCommand::SetMiterLimit(limit));
        }
        self
    }
    
    /// Set global alpha (with deduplication)
    pub fn set_global_alpha(&mut self, alpha: f32) -> &mut Self {
        if self.state.global_alpha != Some(alpha) {
            self.state.global_alpha = Some(alpha);
            self.buffer.push_2d(Canvas2DCommand::SetGlobalAlpha(alpha));
        }
        self
    }
    
    /// Set font
    pub fn set_font(&mut self, family: &str, size: f32, bold: bool, italic: bool) -> &mut Self {
        let needs_update = self.state.font_family.as_deref() != Some(family)
            || self.state.font_size != Some(size);
        
        if needs_update {
            self.state.font_family = Some(family.to_string());
            self.state.font_size = Some(size);
            self.buffer.push_2d(Canvas2DCommand::SetFont {
                family: family.to_string(),
                size,
                bold,
                italic,
            });
        }
        self
    }
    
    /// Set text alignment
    pub fn set_text_align(&mut self, align: u8) -> &mut Self {
        if self.state.text_align != Some(align) {
            self.state.text_align = Some(align);
            self.buffer.push_2d(Canvas2DCommand::SetTextAlign(align));
        }
        self
    }
    
    /// Set text baseline
    pub fn set_text_baseline(&mut self, baseline: u8) -> &mut Self {
        if self.state.text_baseline != Some(baseline) {
            self.state.text_baseline = Some(baseline);
            self.buffer.push_2d(Canvas2DCommand::SetTextBaseline(baseline));
        }
        self
    }
    
    // ========== Transform Commands ==========
    
    /// Set transform matrix
    pub fn set_transform(&mut self, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::SetTransform { a, b, c, d, e, f });
        self
    }
    
    /// Reset transform to identity
    pub fn reset_transform(&mut self) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::ResetTransform);
        self
    }
    
    /// Translate
    pub fn translate(&mut self, x: f32, y: f32) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::Translate { x, y });
        self
    }
    
    /// Rotate
    pub fn rotate(&mut self, angle: f32) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::Rotate(angle));
        self
    }
    
    /// Scale
    pub fn scale(&mut self, x: f32, y: f32) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::Scale { x, y });
        self
    }
    
    // ========== Path Commands ==========
    
    /// Begin a new path
    pub fn begin_path(&mut self) -> &mut Self {
        self.in_path = true;
        self.buffer.push_2d(Canvas2DCommand::BeginPath);
        self
    }
    
    /// Close current subpath
    pub fn close_path(&mut self) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::ClosePath);
        self
    }
    
    /// Move to point
    pub fn move_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::MoveTo { x, y });
        self
    }
    
    /// Line to point
    pub fn line_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::LineTo { x, y });
        self
    }
    
    /// Quadratic curve to
    pub fn quadratic_curve_to(&mut self, cpx: f32, cpy: f32, x: f32, y: f32) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::QuadraticCurveTo { cpx, cpy, x, y });
        self
    }
    
    /// Bezier curve to
    pub fn bezier_curve_to(
        &mut self,
        cp1x: f32, cp1y: f32,
        cp2x: f32, cp2y: f32,
        x: f32, y: f32,
    ) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::BezierCurveTo {
            cp1x, cp1y, cp2x, cp2y, x, y,
        });
        self
    }
    
    /// Arc
    pub fn arc(
        &mut self,
        x: f32, y: f32,
        radius: f32,
        start_angle: f32, end_angle: f32,
        ccw: bool,
    ) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::Arc {
            x, y, radius, start_angle, end_angle, ccw,
        });
        self
    }
    
    /// Arc to
    pub fn arc_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, radius: f32) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::ArcTo { x1, y1, x2, y2, radius });
        self
    }
    
    /// Rectangle path
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::Rect { x, y, w, h });
        self
    }
    
    /// Ellipse
    pub fn ellipse(
        &mut self,
        x: f32, y: f32,
        rx: f32, ry: f32,
        rotation: f32,
        start: f32, end: f32,
        ccw: bool,
    ) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::Ellipse {
            x, y, rx, ry, rotation, start, end, ccw,
        });
        self
    }
    
    // ========== Drawing Commands ==========
    
    /// Fill current path
    pub fn fill(&mut self) -> &mut Self {
        self.in_path = false;
        self.buffer.push_2d(Canvas2DCommand::Fill);
        self
    }
    
    /// Stroke current path
    pub fn stroke(&mut self) -> &mut Self {
        self.in_path = false;
        self.buffer.push_2d(Canvas2DCommand::Stroke);
        self
    }
    
    /// Set clip to current path
    pub fn clip(&mut self) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::Clip);
        self
    }
    
    /// Fill rectangle (optimized, no path needed)
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::FillRect { x, y, w, h });
        self.buffer.mark_dirty(DirtyRect::new(x, y, w, h));
        self
    }
    
    /// Stroke rectangle
    pub fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32) -> &mut Self {
        let lw = self.state.line_width.unwrap_or(1.0);
        self.buffer.push_2d(Canvas2DCommand::StrokeRect { x, y, w, h });
        self.buffer.mark_dirty(DirtyRect::new(x - lw, y - lw, w + 2.0 * lw, h + 2.0 * lw));
        self
    }
    
    /// Clear rectangle
    pub fn clear_rect(&mut self, x: f32, y: f32, w: f32, h: f32) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::ClearRect { x, y, w, h });
        self.buffer.mark_dirty(DirtyRect::new(x, y, w, h));
        self
    }
    
    /// Fill text
    pub fn fill_text(&mut self, text: &str, x: f32, y: f32, max_width: Option<f32>) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::FillText {
            text: text.to_string(),
            x, y,
            max_width,
        });
        self
    }
    
    /// Stroke text
    pub fn stroke_text(&mut self, text: &str, x: f32, y: f32, max_width: Option<f32>) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::StrokeText {
            text: text.to_string(),
            x, y,
            max_width,
        });
        self
    }
    
    /// Draw image
    pub fn draw_image(
        &mut self,
        image_id: u32,
        sx: f32, sy: f32, sw: f32, sh: f32,
        dx: f32, dy: f32, dw: f32, dh: f32,
    ) -> &mut Self {
        self.buffer.push_2d(Canvas2DCommand::DrawImage {
            image_id, sx, sy, sw, sh, dx, dy, dw, dh,
        });
        self.buffer.mark_dirty(DirtyRect::new(dx, dy, dw, dh));
        self
    }
    
    /// Batch draw images (optimized for many images)
    pub fn draw_image_batch(&mut self, images: Vec<DrawImageParams>) -> &mut Self {
        // Track dirty regions
        for img in &images {
            self.buffer.mark_dirty(DirtyRect::new(img.dx, img.dy, img.dw, img.dh));
        }
        self.buffer.push_2d(Canvas2DCommand::DrawImageBatch(images));
        self
    }
}

/// Builder for creating image batches
pub struct ImageBatchBuilder {
    images: Vec<DrawImageParams>,
}

impl ImageBatchBuilder {
    pub fn new() -> Self {
        Self { images: Vec::with_capacity(64) }
    }
    
    pub fn with_capacity(capacity: usize) -> Self {
        Self { images: Vec::with_capacity(capacity) }
    }
    
    pub fn add(
        &mut self,
        image_id: u32,
        sx: f32, sy: f32, sw: f32, sh: f32,
        dx: f32, dy: f32, dw: f32, dh: f32,
    ) -> &mut Self {
        self.images.push(DrawImageParams {
            image_id, sx, sy, sw, sh, dx, dy, dw, dh,
        });
        self
    }
    
    pub fn build(self) -> Vec<DrawImageParams> {
        self.images
    }
}

impl Default for ImageBatchBuilder {
    fn default() -> Self {
        Self::new()
    }
}
