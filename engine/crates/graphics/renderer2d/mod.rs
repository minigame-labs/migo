#![allow(dead_code)]

use femtovg::{
    renderer::OpenGl, Canvas as FvCanvas, Color, FontId, ImageId, LineCap, LineJoin, Paint, Path,
    Transform2D,
};
use shared::protocol::render_cmd::{TextAlign, TextBaseline, TextMetrics};

mod font;
mod handler;

pub(crate) use font::*;
pub(crate) use handler::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CanvasLineCap {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CanvasLineJoin {
    Miter,
    Round,
    Bevel,
}

#[derive(Clone)]
pub enum FillStyleKind {
    Color(Color),
    Gradient {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        start_color: Color,
        end_color: Color,
    },
    Pattern {
        image_id: ImageId,
        repeat_x: bool,
        repeat_y: bool,
    },
}

#[derive(Clone)]
pub struct Canvas2DState {
    pub fill_style: FillStyleKind,
    pub stroke_style: Color,
    pub line_width: f32,
    pub line_cap: CanvasLineCap,
    pub line_join: CanvasLineJoin,
    pub miter_limit: f32,
    pub global_alpha: f32,

    pub font_size: f32,
    pub font_id: Option<FontId>,
    pub text_align: TextAlign,
    pub text_baseline: TextBaseline,

    pub transform: Transform2D,
    pub clip_path: Option<Path>,
}

impl Default for Canvas2DState {
    fn default() -> Self {
        Self {
            fill_style: FillStyleKind::Color(Color::rgb(0, 0, 0)),
            stroke_style: Color::rgb(0, 0, 0),
            line_width: 1.0,
            line_cap: CanvasLineCap::Butt,
            line_join: CanvasLineJoin::Miter,
            miter_limit: 10.0,
            global_alpha: 1.0,

            font_size: 16.0,
            font_id: None,
            text_align: TextAlign::default(),
            text_baseline: TextBaseline::default(),

            transform: Transform2D::identity(),
            clip_path: None,
        }
    }
}

pub(crate) struct Canvas2DContext {
    pub canvas: FvCanvas<OpenGl>,
    pub font_manager: FontManager,
    pub state: Canvas2DState,
    pub stack: Vec<Canvas2DState>,
    pub current_path: Path,
    /// Reusable path for rectangle operations (avoids allocation per draw)
    rect_path: Path,
}

impl Canvas2DContext {
    pub(crate) fn new(canvas: FvCanvas<OpenGl>, font_manager: FontManager) -> Self {
        Self {
            canvas,
            font_manager,
            state: Canvas2DState::default(),
            stack: Vec::with_capacity(8),
            current_path: Path::new(),
            rect_path: Path::new(),
        }
    }

    /// Get a reusable path with a single rect, avoiding allocation
    #[inline]
    fn get_rect_path(&mut self, x: f32, y: f32, w: f32, h: f32) -> &Path {
        // Clear and reuse the existing path
        self.rect_path = Path::new();
        self.rect_path.rect(x, y, w, h);
        &self.rect_path
    }

    #[inline]
    fn clamp01(x: f32) -> f32 { x.clamp(0.0, 1.0) }

    #[inline]
    fn apply_global_alpha(mut c: Color, global_alpha: f32) -> Color {
        let ga = Self::clamp01(global_alpha);
        c.a = (Self::clamp01(c.a) * ga).clamp(0.0, 1.0);
        c
    }

    // ========== Path methods ==========
    pub fn begin_path(&mut self) { self.current_path = Path::new(); }
    pub fn close_path(&mut self) { self.current_path.close(); }
    pub fn move_to(&mut self, x: f32, y: f32) { self.current_path.move_to(x, y); }
    pub fn line_to(&mut self, x: f32, y: f32) { self.current_path.line_to(x, y); }
    pub fn quadratic_curve_to(&mut self, cpx: f32, cpy: f32, x: f32, y: f32) {
        self.current_path.quad_to(cpx, cpy, x, y);
    }
    pub fn bezier_curve_to(&mut self, cp1x: f32, cp1y: f32, cp2x: f32, cp2y: f32, x: f32, y: f32) {
        self.current_path.bezier_to(cp1x, cp1y, cp2x, cp2y, x, y);
    }
    pub fn arc(&mut self, x: f32, y: f32, radius: f32, start_angle: f32, end_angle: f32, ccw: bool) {
        let solidity = if ccw { femtovg::Solidity::Hole } else { femtovg::Solidity::Solid };
        self.current_path.arc(x, y, radius, start_angle, end_angle, solidity);
    }
    pub fn arc_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, radius: f32) {
        self.current_path.arc_to(x1, y1, x2, y2, radius);
    }
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32) { self.current_path.rect(x, y, w, h); }
    pub fn ellipse(&mut self, x: f32, y: f32, rx: f32, ry: f32, rot: f32, sa: f32, ea: f32, ccw: bool) {
        self.canvas.save();
        self.canvas.translate(x, y);
        self.canvas.rotate(rot);
        self.canvas.scale(1.0, ry / rx.max(0.001));
        let solidity = if ccw { femtovg::Solidity::Hole } else { femtovg::Solidity::Solid };
        self.current_path.arc(0.0, 0.0, rx, sa, ea, solidity);
        self.canvas.restore();
    }

    // ========== Style setters ==========
    pub fn set_fill_style_color(&mut self, color: Color) { self.state.fill_style = FillStyleKind::Color(color); }
    pub fn set_stroke_style_color(&mut self, color: Color) { self.state.stroke_style = color; }
    pub fn set_line_width(&mut self, w: f32) { self.state.line_width = w.max(0.0); }
    pub fn set_line_cap(&mut self, cap: u8) {
        self.state.line_cap = match cap { 1 => CanvasLineCap::Round, 2 => CanvasLineCap::Square, _ => CanvasLineCap::Butt };
    }
    pub fn set_line_join(&mut self, join: u8) {
        self.state.line_join = match join { 1 => CanvasLineJoin::Round, 2 => CanvasLineJoin::Bevel, _ => CanvasLineJoin::Miter };
    }
    pub fn set_miter_limit(&mut self, limit: f32) { self.state.miter_limit = limit.max(0.0); }
    pub fn set_global_alpha(&mut self, alpha: f32) { self.state.global_alpha = alpha.clamp(0.0, 1.0); }
    pub fn set_text_align(&mut self, align: TextAlign) { self.state.text_align = align; }
    pub fn set_text_baseline(&mut self, baseline: TextBaseline) { self.state.text_baseline = baseline; }

    // ========== State methods ==========
    pub fn save(&mut self) { self.stack.push(self.state.clone()); self.canvas.save(); }
    pub fn restore(&mut self) { if let Some(s) = self.stack.pop() { self.state = s; self.canvas.restore(); } }

    // ========== Drawing methods ==========
    pub fn fill(&mut self) {
        if self.state.clip_path.is_some() { self.canvas.save(); }
        let paint = self.build_fill_paint();
        self.canvas.fill_path(&self.current_path, &paint);
        if self.state.clip_path.is_some() { self.canvas.restore(); }
    }

    pub fn stroke(&mut self) {
        if self.state.clip_path.is_some() { self.canvas.save(); }
        let stroke = Self::apply_global_alpha(self.state.stroke_style, self.state.global_alpha);
        let paint = Paint::color(stroke)
            .with_line_width(self.state.line_width)
            .with_line_cap(match self.state.line_cap { CanvasLineCap::Butt => LineCap::Butt, CanvasLineCap::Round => LineCap::Round, CanvasLineCap::Square => LineCap::Square })
            .with_line_join(match self.state.line_join { CanvasLineJoin::Miter => LineJoin::Miter, CanvasLineJoin::Round => LineJoin::Round, CanvasLineJoin::Bevel => LineJoin::Bevel })
            .with_miter_limit(self.state.miter_limit);
        self.canvas.stroke_path(&self.current_path, &paint);
        if self.state.clip_path.is_some() { self.canvas.restore(); }
    }

    pub fn clip(&mut self) { self.state.clip_path = Some(self.current_path.clone()); }

    // ========== Rectangle methods ==========
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if w <= 0.0 || h <= 0.0 { return; }
        // Reuse rect_path to avoid allocation
        self.rect_path = Path::new();
        self.rect_path.rect(x, y, w, h);
        let paint = self.build_fill_paint();
        self.canvas.fill_path(&self.rect_path, &paint);
    }

    pub fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if w <= 0.0 || h <= 0.0 { return; }
        // Reuse rect_path to avoid allocation
        self.rect_path = Path::new();
        self.rect_path.rect(x, y, w, h);
        let stroke = Self::apply_global_alpha(self.state.stroke_style, self.state.global_alpha);
        let paint = Paint::color(stroke)
            .with_line_width(self.state.line_width)
            .with_line_cap(match self.state.line_cap { CanvasLineCap::Butt => LineCap::Butt, CanvasLineCap::Round => LineCap::Round, CanvasLineCap::Square => LineCap::Square })
            .with_line_join(match self.state.line_join { CanvasLineJoin::Miter => LineJoin::Miter, CanvasLineJoin::Round => LineJoin::Round, CanvasLineJoin::Bevel => LineJoin::Bevel })
            .with_miter_limit(self.state.miter_limit);
        self.canvas.stroke_path(&self.rect_path, &paint);
    }

    pub fn clear_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if w <= 0.0 || h <= 0.0 { return; }
        self.canvas.save_with(|c| {
            c.reset_transform();
            c.clear_rect(x.max(0.0) as u32, y.max(0.0) as u32, w.max(0.0) as u32, h.max(0.0) as u32, Color::rgba(0, 0, 0, 0));
        });
    }

    // ========== Transform methods ==========
    pub fn set_transform(&mut self, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) {
        self.state.transform = Transform2D::new(a, b, c, d, e, f);
        self.canvas.set_transform(&self.state.transform);
    }
    pub fn reset_transform(&mut self) { self.state.transform = Transform2D::identity(); self.canvas.reset_transform(); }
    pub fn translate(&mut self, x: f32, y: f32) {
        self.canvas.translate(x, y);
        self.state.transform.premultiply(&Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y));
    }
    pub fn rotate(&mut self, angle: f32) {
        self.canvas.rotate(angle);
        let (cos, sin) = (angle.cos(), angle.sin());
        self.state.transform.premultiply(&Transform2D::new(cos, sin, -sin, cos, 0.0, 0.0));
    }
    pub fn scale(&mut self, x: f32, y: f32) {
        self.canvas.scale(x, y);
        self.state.transform.premultiply(&Transform2D::new(x, 0.0, 0.0, y, 0.0, 0.0));
    }

    // ========== Text methods ==========
    pub fn fill_text(&mut self, text: &str, x: f32, y: f32, _max_width: f32) {
        let mut paint = self.build_fill_paint();
        if let Some(font_id) = self.state.font_id { paint.set_font(&[font_id]); }
        paint.set_font_size(self.state.font_size);
        paint.set_text_align(match self.state.text_align { TextAlign::Start | TextAlign::Left => femtovg::Align::Left, TextAlign::End | TextAlign::Right => femtovg::Align::Right, TextAlign::Center => femtovg::Align::Center });
        paint.set_text_baseline(match self.state.text_baseline { TextBaseline::Top | TextBaseline::Hanging => femtovg::Baseline::Top, TextBaseline::Middle => femtovg::Baseline::Middle, TextBaseline::Alphabetic => femtovg::Baseline::Alphabetic, TextBaseline::Ideographic | TextBaseline::Bottom => femtovg::Baseline::Bottom });
        let _ = self.canvas.fill_text(x, y, text, &paint);
    }

    pub fn stroke_text(&mut self, text: &str, x: f32, y: f32, _max_width: f32) {
        let stroke = Self::apply_global_alpha(self.state.stroke_style, self.state.global_alpha);
        let mut paint = Paint::color(stroke).with_line_width(self.state.line_width);
        if let Some(font_id) = self.state.font_id { paint.set_font(&[font_id]); }
        paint.set_font_size(self.state.font_size);
        paint.set_text_align(match self.state.text_align { TextAlign::Start | TextAlign::Left => femtovg::Align::Left, TextAlign::End | TextAlign::Right => femtovg::Align::Right, TextAlign::Center => femtovg::Align::Center });
        paint.set_text_baseline(match self.state.text_baseline { TextBaseline::Top | TextBaseline::Hanging => femtovg::Baseline::Top, TextBaseline::Middle => femtovg::Baseline::Middle, TextBaseline::Alphabetic => femtovg::Baseline::Alphabetic, TextBaseline::Ideographic | TextBaseline::Bottom => femtovg::Baseline::Bottom });
        let _ = self.canvas.fill_text(x, y, text, &paint);
    }

    pub fn measure_text(&mut self, text: &str) -> TextMetrics {
        let mut paint = self.build_fill_paint();
        if let Some(font_id) = self.state.font_id { paint.set_font(&[font_id]); }
        paint.set_font_size(self.state.font_size);

        // Get actual font metrics (use methods, not fields)
        let (ascender, descender) = self.canvas.measure_font(&paint)
            .map(|m| (m.ascender(), m.descender()))
            .unwrap_or((self.state.font_size * 0.8, self.state.font_size * -0.2));

        match self.canvas.measure_text(0.0, 0.0, text, &paint) {
            Ok(m) => TextMetrics {
                width: m.width(),
                actual_bounding_box_left: 0.0,
                actual_bounding_box_right: m.width(),
                actual_bounding_box_ascent: ascender,
                actual_bounding_box_descent: -descender, // descender is negative
                font_bounding_box_ascent: ascender,
                font_bounding_box_descent: -descender,
            },
            Err(_) => TextMetrics { width: 0.0, actual_bounding_box_left: 0.0, actual_bounding_box_right: 0.0, actual_bounding_box_ascent: 0.0, actual_bounding_box_descent: 0.0, font_bounding_box_ascent: 0.0, font_bounding_box_descent: 0.0 },
        }
    }

    // ========== Image methods ==========
    pub fn draw_image_rect(&mut self, image_id: ImageId, img_size: (f32, f32), sx: f32, sy: f32, sw: f32, sh: f32, dx: f32, dy: f32, dw: f32, dh: f32) {
        let (img_w, img_h) = img_size;
        let (mut sx, mut sy, mut sw, mut sh) = (sx, sy, sw, sh);
        if sw <= 0.0 { sw = img_w; }
        if sh <= 0.0 { sh = img_h; }
        if sx < 0.0 { sx = 0.0; }
        if sy < 0.0 { sy = 0.0; }
        if sx + sw > img_w { sw = (img_w - sx).max(0.0); }
        if sy + sh > img_h { sh = (img_h - sy).max(0.0); }
        if sw <= 0.0 || sh <= 0.0 { return; }
        let (mut dw, mut dh) = (dw, dh);
        if dw <= 0.0 { dw = sw; }
        if dh <= 0.0 { dh = sh; }
        if dw <= 0.0 || dh <= 0.0 { return; }
        let (scale_x, scale_y) = (dw / sw, dh / sh);
        let ga = Self::clamp01(self.state.global_alpha);
        let source_is_whole = sx == 0.0 && sy == 0.0 && (sw - img_w).abs() < f32::EPSILON && (sh - img_h).abs() < f32::EPSILON;

        // Reuse rect_path to avoid allocation per draw call
        self.rect_path = Path::new();
        self.rect_path.rect(dx, dy, dw, dh);

        if source_is_whole {
            let paint = Paint::image(image_id, dx, dy, dw, dh, 0.0, ga).with_anti_alias(scale_x != 1.0 || scale_y != 1.0);
            self.canvas.fill_path(&self.rect_path, &paint);
        } else {
            let (draw_x, draw_y, draw_w, draw_h) = (dx - sx * scale_x, dy - sy * scale_y, img_w * scale_x, img_h * scale_y);
            let paint = Paint::image(image_id, draw_x, draw_y, draw_w, draw_h, 0.0, ga).with_anti_alias(scale_x != 1.0 || scale_y != 1.0);
            self.canvas.fill_path(&self.rect_path, &paint);
        }
    }

    fn build_fill_paint(&self) -> Paint {
        let ga = Self::clamp01(self.state.global_alpha);
        match &self.state.fill_style {
            FillStyleKind::Color(c) => Paint::color(Self::apply_global_alpha(*c, ga)),
            FillStyleKind::Gradient { x0, y0, x1, y1, start_color, end_color } => Paint::linear_gradient(*x0, *y0, *x1, *y1, Self::apply_global_alpha(*start_color, ga), Self::apply_global_alpha(*end_color, ga)),
            FillStyleKind::Pattern { image_id, .. } => Paint::image(*image_id, 0.0, 0.0, 1.0, 1.0, 0.0, ga),
        }
    }

    pub fn flush(&mut self) { self.canvas.flush(); }
}
