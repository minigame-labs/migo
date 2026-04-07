#![allow(dead_code)]

use femtovg::{
    renderer::OpenGl, Canvas as FvCanvas, Color, FontId, ImageId, LineCap, LineJoin, Paint, Path,
    Transform2D,
};
use shared::protocol::color::Color as SharedColor;
use shared::protocol::render_cmd::{
    GradientStop, GradientType, TextAlign, TextBaseline, TextMetrics,
};

/// Convert a shared protocol `Color` to femtovg's `Color`.
#[inline]
fn to_fv_color(c: SharedColor) -> Color {
    Color::rgbaf(c.r, c.g, c.b, c.a)
}

pub(crate) mod display_list;
mod font;
pub(crate) mod handler;
mod layer_cache;
pub(crate) mod sprite_batch;

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
    LinearGradient {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        stops: Vec<(f32, Color)>,
    },
    RadialGradient {
        x0: f32,
        y0: f32,
        r0: f32,
        x1: f32,
        y1: f32,
        r1: f32,
        stops: Vec<(f32, Color)>,
    },
    ConicGradient {
        cx: f32,
        cy: f32,
        stops: Vec<(f32, Color)>,
    },
    Pattern {
        image_id: ImageId,
        repeat_x: bool,
        repeat_y: bool,
        width: f32,
        height: f32,
    },
}

#[derive(Clone)]
pub struct Canvas2DState {
    pub fill_style: FillStyleKind,
    pub stroke_style: FillStyleKind,
    pub line_width: f32,
    pub line_cap: CanvasLineCap,
    pub line_join: CanvasLineJoin,
    pub miter_limit: f32,
    pub global_alpha: f32,
    pub composite_op: femtovg::CompositeOperation,
    pub line_dash: Vec<f32>,
    pub line_dash_offset: f32,

    pub font_size: f32,
    pub font_id: Option<FontId>,
    pub font_cache_key: String,
    pub text_align: TextAlign,
    pub text_baseline: TextBaseline,

    pub transform: Transform2D,
    /// Whether a clip region is active (managed via femtovg scissor).
    pub has_clip: bool,

    pub shadow_blur: f32,
    pub shadow_color: Color,
    pub shadow_offset_x: f32,
    pub shadow_offset_y: f32,
}

impl Default for Canvas2DState {
    fn default() -> Self {
        Self {
            fill_style: FillStyleKind::Color(Color::rgb(0, 0, 0)),
            stroke_style: FillStyleKind::Color(Color::rgb(0, 0, 0)),
            line_width: 1.0,
            line_cap: CanvasLineCap::Butt,
            line_join: CanvasLineJoin::Miter,
            miter_limit: 10.0,
            global_alpha: 1.0,
            composite_op: femtovg::CompositeOperation::SourceOver,
            line_dash: Vec::new(),
            line_dash_offset: 0.0,

            font_size: 16.0,
            font_id: None,
            font_cache_key: "16:default:normal:normal".to_string(),
            text_align: TextAlign::default(),
            text_baseline: TextBaseline::default(),

            transform: Transform2D::identity(),
            has_clip: false,

            shadow_blur: 0.0,
            shadow_color: Color::rgba(0, 0, 0, 0),
            shadow_offset_x: 0.0,
            shadow_offset_y: 0.0,
        }
    }
}

pub(crate) struct Canvas2DContext {
    pub canvas: FvCanvas<OpenGl>,
    pub font_manager: FontManager,
    pub state: Canvas2DState,
    pub stack: Vec<Canvas2DState>,
    pub current_path: Path,
    has_current_point: bool,
    /// Bounding box of current path (for clip scissor). Reset on begin_path.
    path_min_x: f32,
    path_min_y: f32,
    path_max_x: f32,
    path_max_y: f32,
    /// Reusable path for rectangle operations (avoids allocation per draw)
    rect_path: Path,
    /// Cached last font string to skip re-parsing identical SetFont calls.
    last_font_str: String,
    last_font_id: Option<FontId>,
    last_font_size: f32,
    last_font_cache_key: String,
}

impl Canvas2DContext {
    pub(crate) fn new(canvas: FvCanvas<OpenGl>, font_manager: FontManager) -> Self {
        Self {
            canvas,
            font_manager,
            state: Canvas2DState::default(),
            stack: Vec::with_capacity(8),
            current_path: Path::new(),
            has_current_point: false,
            path_min_x: f32::MAX,
            path_min_y: f32::MAX,
            path_max_x: f32::MIN,
            path_max_y: f32::MIN,
            rect_path: Path::new(),
            last_font_str: String::new(),
            last_font_id: None,
            last_font_size: 16.0,
            last_font_cache_key: String::new(),
        }
    }

    /// Expand path bounding box to include point (x, y).
    #[inline]
    fn extend_bounds(&mut self, x: f32, y: f32) {
        if x < self.path_min_x {
            self.path_min_x = x;
        }
        if y < self.path_min_y {
            self.path_min_y = y;
        }
        if x > self.path_max_x {
            self.path_max_x = x;
        }
        if y > self.path_max_y {
            self.path_max_y = y;
        }
    }

    #[inline]
    fn reset_bounds(&mut self) {
        self.path_min_x = f32::MAX;
        self.path_min_y = f32::MAX;
        self.path_max_x = f32::MIN;
        self.path_max_y = f32::MIN;
    }

    #[inline]
    fn clamp01(x: f32) -> f32 {
        x.clamp(0.0, 1.0)
    }

    #[inline]
    fn apply_global_alpha(mut c: Color, global_alpha: f32) -> Color {
        let ga = Self::clamp01(global_alpha);
        c.a = (Self::clamp01(c.a) * ga).clamp(0.0, 1.0);
        c
    }

    fn rotate_conic_stops(mut stops: Vec<(f32, Color)>, start_angle: f32) -> Vec<(f32, Color)> {
        if stops.is_empty() {
            return stops;
        }

        let shift = (start_angle / std::f32::consts::TAU).rem_euclid(1.0);
        if shift.abs() <= f32::EPSILON {
            return stops;
        }

        for (offset, _) in &mut stops {
            *offset = (*offset + shift).rem_euclid(1.0);
        }
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        stops
    }

    // ========== Path methods ==========
    pub fn begin_path(&mut self) {
        self.current_path = Path::new();
        self.has_current_point = false;
        self.reset_bounds();
    }
    pub fn close_path(&mut self) {
        self.current_path.close();
    }
    pub fn move_to(&mut self, x: f32, y: f32) {
        self.current_path.move_to(x, y);
        self.has_current_point = true;
        self.extend_bounds(x, y);
    }
    pub fn line_to(&mut self, x: f32, y: f32) {
        self.current_path.line_to(x, y);
        self.has_current_point = true;
        self.extend_bounds(x, y);
    }
    pub fn quadratic_curve_to(&mut self, cpx: f32, cpy: f32, x: f32, y: f32) {
        self.current_path.quad_to(cpx, cpy, x, y);
        self.has_current_point = true;
        self.extend_bounds(cpx, cpy);
        self.extend_bounds(x, y);
    }
    pub fn bezier_curve_to(&mut self, cp1x: f32, cp1y: f32, cp2x: f32, cp2y: f32, x: f32, y: f32) {
        self.current_path.bezier_to(cp1x, cp1y, cp2x, cp2y, x, y);
        self.has_current_point = true;
        self.extend_bounds(cp1x, cp1y);
        self.extend_bounds(cp2x, cp2y);
        self.extend_bounds(x, y);
    }
    pub fn arc(
        &mut self,
        x: f32,
        y: f32,
        radius: f32,
        start_angle: f32,
        end_angle: f32,
        ccw: bool,
    ) {
        let solidity = if ccw {
            femtovg::Solidity::Hole
        } else {
            femtovg::Solidity::Solid
        };
        self.current_path
            .arc(x, y, radius, start_angle, end_angle, solidity);
        self.has_current_point = true;
        // Conservative bounding box for arc
        self.extend_bounds(x - radius, y - radius);
        self.extend_bounds(x + radius, y + radius);
    }
    pub fn arc_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, radius: f32) {
        self.current_path.arc_to(x1, y1, x2, y2, radius);
        self.has_current_point = true;
        // Both control points ±radius in all directions for conservative bbox
        self.extend_bounds(x1 - radius, y1 - radius);
        self.extend_bounds(x1 + radius, y1 + radius);
        self.extend_bounds(x2 - radius, y2 - radius);
        self.extend_bounds(x2 + radius, y2 + radius);
    }
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.current_path.rect(x, y, w, h);
        self.has_current_point = true;
        self.extend_bounds(x, y);
        self.extend_bounds(x + w, y + h);
    }
    pub fn ellipse(
        &mut self,
        x: f32,
        y: f32,
        rx: f32,
        ry: f32,
        rot: f32,
        sa: f32,
        ea: f32,
        ccw: bool,
    ) {
        // Build the ellipse directly on current_path by computing transformed
        // arc points on the CPU.  We cannot use canvas.save/translate/scale
        // because the temporary canvas transform is restored before fill/stroke
        // and Path stores raw vertices — the transform would be lost.
        if rx <= 0.0 || ry <= 0.0 {
            return;
        }

        let (cos_r, sin_r) = (rot.cos(), rot.sin());

        // Determine sweep and step direction
        let mut sweep = ea - sa;
        if ccw {
            if sweep > 0.0 {
                sweep -= std::f32::consts::TAU;
            }
        } else if sweep < 0.0 {
            sweep += std::f32::consts::TAU;
        }

        let max_radius = rx.max(ry).max(1.0);
        // Keep segment arc length around 6px for smoother large ellipses.
        let steps = ((sweep.abs() * max_radius) / 6.0).ceil().clamp(4.0, 256.0) as usize;
        let dt = sweep / steps as f32;

        for i in 0..=steps {
            let t = sa + dt * i as f32;
            // Point on the unit ellipse, then rotate + translate
            let px = rx * t.cos();
            let py = ry * t.sin();
            let fx = x + px * cos_r - py * sin_r;
            let fy = y + px * sin_r + py * cos_r;
            self.extend_bounds(fx, fy);
            if i == 0 {
                if self.has_current_point {
                    self.current_path.line_to(fx, fy);
                } else {
                    self.current_path.move_to(fx, fy);
                }
            } else {
                self.current_path.line_to(fx, fy);
            }
        }
        self.has_current_point = true;
    }

    // ========== Style setters ==========
    pub fn set_fill_style_color(&mut self, color: SharedColor) {
        self.state.fill_style = FillStyleKind::Color(to_fv_color(color));
    }
    pub fn set_stroke_style_color(&mut self, color: SharedColor) {
        self.state.stroke_style = FillStyleKind::Color(to_fv_color(color));
    }
    pub fn set_line_width(&mut self, w: f32) {
        self.state.line_width = w.max(0.0);
    }
    pub fn set_line_cap(&mut self, cap: u8) {
        self.state.line_cap = match cap {
            1 => CanvasLineCap::Round,
            2 => CanvasLineCap::Square,
            _ => CanvasLineCap::Butt,
        };
    }
    pub fn set_line_join(&mut self, join: u8) {
        self.state.line_join = match join {
            1 => CanvasLineJoin::Round,
            2 => CanvasLineJoin::Bevel,
            _ => CanvasLineJoin::Miter,
        };
    }
    pub fn set_miter_limit(&mut self, limit: f32) {
        self.state.miter_limit = limit.max(0.0);
    }
    pub fn set_global_alpha(&mut self, alpha: f32) {
        self.state.global_alpha = alpha.clamp(0.0, 1.0);
    }
    pub fn set_composite_operation(&mut self, op: u8) {
        let comp = match op {
            0 => femtovg::CompositeOperation::SourceOver,
            1 => femtovg::CompositeOperation::SourceIn,
            2 => femtovg::CompositeOperation::SourceOut,
            3 => femtovg::CompositeOperation::Atop,
            4 => femtovg::CompositeOperation::DestinationOver,
            5 => femtovg::CompositeOperation::DestinationIn,
            6 => femtovg::CompositeOperation::DestinationOut,
            7 => femtovg::CompositeOperation::DestinationAtop,
            8 => femtovg::CompositeOperation::Lighter,
            9 => femtovg::CompositeOperation::Copy,
            10 => femtovg::CompositeOperation::Xor,
            _ => femtovg::CompositeOperation::SourceOver,
        };
        self.state.composite_op = comp;
        self.canvas.global_composite_operation(comp);
    }
    pub fn set_line_dash(&mut self, segments: Vec<f32>) {
        self.state.line_dash = segments;
    }
    pub fn set_line_dash_offset(&mut self, offset: f32) {
        self.state.line_dash_offset = offset;
    }
    pub fn set_shadow_blur(&mut self, blur: f32) {
        self.state.shadow_blur = blur.max(0.0);
    }
    pub fn set_shadow_color(&mut self, color: SharedColor) {
        self.state.shadow_color = to_fv_color(color);
    }
    pub fn set_shadow_offset_x(&mut self, offset: f32) {
        self.state.shadow_offset_x = offset;
    }
    pub fn set_shadow_offset_y(&mut self, offset: f32) {
        self.state.shadow_offset_y = offset;
    }
    pub fn set_fill_style_gradient(
        &mut self,
        gradient_type: GradientType,
        x0: f32,
        y0: f32,
        r0: f32,
        x1: f32,
        y1: f32,
        r1: f32,
        stops: Vec<GradientStop>,
    ) {
        if stops.is_empty() {
            return;
        }

        let mut stop_pairs: Vec<(f32, Color)> = stops
            .into_iter()
            .map(|s| (s.offset.clamp(0.0, 1.0), to_fv_color(s.color)))
            .collect();

        stop_pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

        if stop_pairs.len() == 1 {
            self.state.fill_style = FillStyleKind::Color(stop_pairs[0].1);
            return;
        }

        self.state.fill_style = match gradient_type {
            GradientType::Linear => FillStyleKind::LinearGradient {
                x0,
                y0,
                x1,
                y1,
                stops: stop_pairs,
            },
            GradientType::Radial => FillStyleKind::RadialGradient {
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                stops: stop_pairs,
            },
            GradientType::Conic => FillStyleKind::ConicGradient {
                cx: x0,
                cy: y0,
                stops: Self::rotate_conic_stops(stop_pairs, x1),
            },
        };
    }
    pub fn set_stroke_style_gradient(
        &mut self,
        gradient_type: GradientType,
        x0: f32,
        y0: f32,
        r0: f32,
        x1: f32,
        y1: f32,
        r1: f32,
        stops: Vec<GradientStop>,
    ) {
        if stops.is_empty() {
            return;
        }

        let mut stop_pairs: Vec<(f32, Color)> = stops
            .into_iter()
            .map(|s| (s.offset.clamp(0.0, 1.0), to_fv_color(s.color)))
            .collect();

        stop_pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

        if stop_pairs.len() == 1 {
            self.state.stroke_style = FillStyleKind::Color(stop_pairs[0].1);
            return;
        }

        self.state.stroke_style = match gradient_type {
            GradientType::Linear => FillStyleKind::LinearGradient {
                x0,
                y0,
                x1,
                y1,
                stops: stop_pairs,
            },
            GradientType::Radial => FillStyleKind::RadialGradient {
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                stops: stop_pairs,
            },
            GradientType::Conic => FillStyleKind::ConicGradient {
                cx: x0,
                cy: y0,
                stops: Self::rotate_conic_stops(stop_pairs, x1),
            },
        };
    }

    pub fn set_fill_style_pattern(
        &mut self,
        image_id: ImageId,
        repeat_x: bool,
        repeat_y: bool,
        width: f32,
        height: f32,
    ) {
        self.state.fill_style = FillStyleKind::Pattern {
            image_id,
            repeat_x,
            repeat_y,
            width,
            height,
        };
    }

    pub fn set_stroke_style_pattern(
        &mut self,
        image_id: ImageId,
        repeat_x: bool,
        repeat_y: bool,
        width: f32,
        height: f32,
    ) {
        self.state.stroke_style = FillStyleKind::Pattern {
            image_id,
            repeat_x,
            repeat_y,
            width,
            height,
        };
    }

    pub fn set_text_align(&mut self, align: TextAlign) {
        self.state.text_align = align;
    }
    pub fn set_text_baseline(&mut self, baseline: TextBaseline) {
        self.state.text_baseline = baseline;
    }

    // ========== State methods ==========
    pub fn save(&mut self) {
        self.stack.push(self.state.clone());
        self.canvas.save();
    }
    pub fn restore(&mut self) {
        if let Some(s) = self.stack.pop() {
            self.state = s;
            self.canvas.restore();
        }
    }

    // ========== Drawing methods ==========
    pub fn fill(&mut self) {
        self.draw_path_shadow_fill();
        let paint = self.build_fill_paint();
        self.canvas.fill_path(&self.current_path, &paint);
    }

    pub fn stroke(&mut self) {
        self.draw_path_shadow_stroke();
        let paint = self.build_stroke_paint();
        self.canvas.stroke_path(&self.current_path, &paint);
    }

    /// Apply the current path as a clip region using femtovg scissor.
    /// Uses the tracked bounding box of the path — exact for rectangles,
    /// conservative approximation for curves.
    pub fn clip(&mut self) {
        if self.path_min_x < self.path_max_x && self.path_min_y < self.path_max_y {
            let x = self.path_min_x;
            let y = self.path_min_y;
            let w = self.path_max_x - self.path_min_x;
            let h = self.path_max_y - self.path_min_y;
            self.canvas.intersect_scissor(x, y, w, h);
        }
        self.state.has_clip = true;
    }

    // ========== Shadow helper ==========
    fn has_shadow(&self) -> bool {
        self.state.shadow_color.a > 0.0
    }

    fn shadow_color_with_alpha(&self) -> Color {
        Self::apply_global_alpha(self.state.shadow_color, self.state.global_alpha)
    }

    fn build_shadow_stroke_paint(&self) -> Paint {
        Paint::color(self.shadow_color_with_alpha())
            .with_line_width(self.state.line_width)
            .with_line_cap(match self.state.line_cap {
                CanvasLineCap::Butt => LineCap::Butt,
                CanvasLineCap::Round => LineCap::Round,
                CanvasLineCap::Square => LineCap::Square,
            })
            .with_line_join(match self.state.line_join {
                CanvasLineJoin::Miter => LineJoin::Miter,
                CanvasLineJoin::Round => LineJoin::Round,
                CanvasLineJoin::Bevel => LineJoin::Bevel,
            })
            .with_miter_limit(self.state.miter_limit)
    }

    fn draw_rect_shadow(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if !self.has_shadow() {
            return;
        }
        let blur = self.state.shadow_blur;
        let ox = self.state.shadow_offset_x;
        let oy = self.state.shadow_offset_y;
        let sc = self.shadow_color_with_alpha();

        if blur <= f32::EPSILON {
            self.rect_path = Path::new();
            self.rect_path.rect(x + ox, y + oy, w, h);
            self.canvas.fill_path(&self.rect_path, &Paint::color(sc));
            return;
        }

        let feather = blur * 2.0;
        let spread = blur;
        let paint = Paint::box_gradient(
            x + ox - spread,
            y + oy - spread,
            w + spread * 2.0,
            h + spread * 2.0,
            0.0,
            feather,
            sc,
            Color::rgba(0, 0, 0, 0),
        );
        let margin = blur * 3.0;
        self.rect_path = Path::new();
        self.rect_path.rect(
            x + ox - spread - margin,
            y + oy - spread - margin,
            w + (spread + margin) * 2.0,
            h + (spread + margin) * 2.0,
        );
        self.canvas.fill_path(&self.rect_path, &paint);
    }

    fn draw_path_shadow_fill(&mut self) {
        if !self.has_shadow() {
            return;
        }
        self.canvas.save();
        self.canvas
            .translate(self.state.shadow_offset_x, self.state.shadow_offset_y);
        let shadow_paint = Paint::color(self.shadow_color_with_alpha());
        self.canvas.fill_path(&self.current_path, &shadow_paint);
        self.canvas.restore();
    }

    fn draw_path_shadow_stroke(&mut self) {
        if !self.has_shadow() {
            return;
        }
        self.canvas.save();
        self.canvas
            .translate(self.state.shadow_offset_x, self.state.shadow_offset_y);
        let shadow_paint = self.build_shadow_stroke_paint();
        self.canvas.stroke_path(&self.current_path, &shadow_paint);
        self.canvas.restore();
    }

    // ========== Rectangle methods ==========
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        self.draw_rect_shadow(x, y, w, h);
        // Reuse rect_path to avoid allocation
        self.rect_path = Path::new();
        self.rect_path.rect(x, y, w, h);
        let paint = self.build_fill_paint();
        self.canvas.fill_path(&self.rect_path, &paint);
    }

    pub fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        self.draw_rect_shadow(x, y, w, h);
        // Reuse rect_path to avoid allocation
        self.rect_path = Path::new();
        self.rect_path.rect(x, y, w, h);
        let paint = self.build_stroke_paint();
        self.canvas.stroke_path(&self.rect_path, &paint);
    }

    pub fn clear_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        // Use DestinationOut composite with a fully-opaque fill to erase
        // the rectangle.  This respects the current CTM and handles negative
        // coordinates correctly, unlike the previous clear_rect(u32) approach.
        self.canvas.save();
        self.canvas
            .global_composite_operation(femtovg::CompositeOperation::DestinationOut);
        self.rect_path = Path::new();
        self.rect_path.rect(x, y, w, h);
        self.canvas
            .fill_path(&self.rect_path, &Paint::color(Color::white()));
        self.canvas.restore();
    }

    // ========== Transform methods ==========
    pub fn set_transform(&mut self, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) {
        self.state.transform = Transform2D::new(a, b, c, d, e, f);
        self.canvas.set_transform(&self.state.transform);
    }
    pub fn reset_transform(&mut self) {
        self.state.transform = Transform2D::identity();
        self.canvas.reset_transform();
    }
    pub fn translate(&mut self, x: f32, y: f32) {
        self.canvas.translate(x, y);
        self.state
            .transform
            .premultiply(&Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y));
    }
    pub fn rotate(&mut self, angle: f32) {
        self.canvas.rotate(angle);
        let (cos, sin) = (angle.cos(), angle.sin());
        self.state
            .transform
            .premultiply(&Transform2D::new(cos, sin, -sin, cos, 0.0, 0.0));
    }
    pub fn scale(&mut self, x: f32, y: f32) {
        self.canvas.scale(x, y);
        self.state
            .transform
            .premultiply(&Transform2D::new(x, 0.0, 0.0, y, 0.0, 0.0));
    }

    // ========== Text methods ==========
    pub fn fill_text(&mut self, text: &str, x: f32, y: f32, _max_width: f32) {
        if self.has_shadow() {
            let mut shadow_paint = Paint::color(self.shadow_color_with_alpha());
            if let Some(font_id) = self.state.font_id {
                shadow_paint.set_font(&[font_id]);
            }
            shadow_paint.set_font_size(self.state.font_size);
            shadow_paint.set_text_align(match self.state.text_align {
                TextAlign::Start | TextAlign::Left => femtovg::Align::Left,
                TextAlign::End | TextAlign::Right => femtovg::Align::Right,
                TextAlign::Center => femtovg::Align::Center,
            });
            shadow_paint.set_text_baseline(match self.state.text_baseline {
                TextBaseline::Top | TextBaseline::Hanging => femtovg::Baseline::Top,
                TextBaseline::Middle => femtovg::Baseline::Middle,
                TextBaseline::Alphabetic => femtovg::Baseline::Alphabetic,
                TextBaseline::Ideographic | TextBaseline::Bottom => femtovg::Baseline::Bottom,
            });
            let _ = self.canvas.fill_text(
                x + self.state.shadow_offset_x,
                y + self.state.shadow_offset_y,
                text,
                &shadow_paint,
            );
        }

        let mut paint = self.build_fill_paint();
        if let Some(font_id) = self.state.font_id {
            paint.set_font(&[font_id]);
        }
        paint.set_font_size(self.state.font_size);
        paint.set_text_align(match self.state.text_align {
            TextAlign::Start | TextAlign::Left => femtovg::Align::Left,
            TextAlign::End | TextAlign::Right => femtovg::Align::Right,
            TextAlign::Center => femtovg::Align::Center,
        });
        paint.set_text_baseline(match self.state.text_baseline {
            TextBaseline::Top | TextBaseline::Hanging => femtovg::Baseline::Top,
            TextBaseline::Middle => femtovg::Baseline::Middle,
            TextBaseline::Alphabetic => femtovg::Baseline::Alphabetic,
            TextBaseline::Ideographic | TextBaseline::Bottom => femtovg::Baseline::Bottom,
        });
        let _ = self.canvas.fill_text(x, y, text, &paint);
    }

    pub fn stroke_text(&mut self, text: &str, x: f32, y: f32, _max_width: f32) {
        if self.has_shadow() {
            let mut shadow_paint = self.build_shadow_stroke_paint();
            if let Some(font_id) = self.state.font_id {
                shadow_paint.set_font(&[font_id]);
            }
            shadow_paint.set_font_size(self.state.font_size);
            shadow_paint.set_text_align(match self.state.text_align {
                TextAlign::Start | TextAlign::Left => femtovg::Align::Left,
                TextAlign::End | TextAlign::Right => femtovg::Align::Right,
                TextAlign::Center => femtovg::Align::Center,
            });
            shadow_paint.set_text_baseline(match self.state.text_baseline {
                TextBaseline::Top | TextBaseline::Hanging => femtovg::Baseline::Top,
                TextBaseline::Middle => femtovg::Baseline::Middle,
                TextBaseline::Alphabetic => femtovg::Baseline::Alphabetic,
                TextBaseline::Ideographic | TextBaseline::Bottom => femtovg::Baseline::Bottom,
            });
            let _ = self.canvas.stroke_text(
                x + self.state.shadow_offset_x,
                y + self.state.shadow_offset_y,
                text,
                &shadow_paint,
            );
        }

        let mut paint = self.build_stroke_paint();
        if let Some(font_id) = self.state.font_id {
            paint.set_font(&[font_id]);
        }
        paint.set_font_size(self.state.font_size);
        paint.set_text_align(match self.state.text_align {
            TextAlign::Start | TextAlign::Left => femtovg::Align::Left,
            TextAlign::End | TextAlign::Right => femtovg::Align::Right,
            TextAlign::Center => femtovg::Align::Center,
        });
        paint.set_text_baseline(match self.state.text_baseline {
            TextBaseline::Top | TextBaseline::Hanging => femtovg::Baseline::Top,
            TextBaseline::Middle => femtovg::Baseline::Middle,
            TextBaseline::Alphabetic => femtovg::Baseline::Alphabetic,
            TextBaseline::Ideographic | TextBaseline::Bottom => femtovg::Baseline::Bottom,
        });
        let _ = self.canvas.stroke_text(x, y, text, &paint);
    }

    pub fn measure_text(&mut self, text: &str) -> TextMetrics {
        let mut paint = self.build_fill_paint();
        if let Some(font_id) = self.state.font_id {
            paint.set_font(&[font_id]);
        }
        paint.set_font_size(self.state.font_size);

        // Get actual font metrics (use methods, not fields)
        let (ascender, descender) = self
            .canvas
            .measure_font(&paint)
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
            Err(_) => TextMetrics {
                width: 0.0,
                actual_bounding_box_left: 0.0,
                actual_bounding_box_right: 0.0,
                actual_bounding_box_ascent: 0.0,
                actual_bounding_box_descent: 0.0,
                font_bounding_box_ascent: 0.0,
                font_bounding_box_descent: 0.0,
            },
        }
    }

    // ========== Image methods ==========
    pub fn draw_image_rect(
        &mut self,
        image_id: ImageId,
        img_size: (f32, f32),
        sx: f32,
        sy: f32,
        sw: f32,
        sh: f32,
        dx: f32,
        dy: f32,
        dw: f32,
        dh: f32,
    ) {
        let (img_w, img_h) = img_size;
        let (mut sx, mut sy, mut sw, mut sh) = (sx, sy, sw, sh);
        if sw <= 0.0 {
            sw = img_w;
        }
        if sh <= 0.0 {
            sh = img_h;
        }
        if sx < 0.0 {
            sx = 0.0;
        }
        if sy < 0.0 {
            sy = 0.0;
        }
        if sx + sw > img_w {
            sw = (img_w - sx).max(0.0);
        }
        if sy + sh > img_h {
            sh = (img_h - sy).max(0.0);
        }
        if sw <= 0.0 || sh <= 0.0 {
            return;
        }
        let (mut dw, mut dh) = (dw, dh);
        if dw <= 0.0 {
            dw = sw;
        }
        if dh <= 0.0 {
            dh = sh;
        }
        if dw <= 0.0 || dh <= 0.0 {
            return;
        }

        self.draw_rect_shadow(dx, dy, dw, dh);

        let (scale_x, scale_y) = (dw / sw, dh / sh);
        let ga = Self::clamp01(self.state.global_alpha);
        let source_is_whole = sx == 0.0
            && sy == 0.0
            && (sw - img_w).abs() < f32::EPSILON
            && (sh - img_h).abs() < f32::EPSILON;

        // Reuse rect_path to avoid allocation per draw call
        self.rect_path = Path::new();
        self.rect_path.rect(dx, dy, dw, dh);

        if source_is_whole {
            let paint = Paint::image(image_id, dx, dy, dw, dh, 0.0, ga)
                .with_anti_alias(scale_x != 1.0 || scale_y != 1.0);
            self.canvas.fill_path(&self.rect_path, &paint);
        } else {
            let (draw_x, draw_y, draw_w, draw_h) = (
                dx - sx * scale_x,
                dy - sy * scale_y,
                img_w * scale_x,
                img_h * scale_y,
            );
            let paint = Paint::image(image_id, draw_x, draw_y, draw_w, draw_h, 0.0, ga)
                .with_anti_alias(scale_x != 1.0 || scale_y != 1.0);
            self.canvas.fill_path(&self.rect_path, &paint);
        }
    }

    fn build_fill_paint(&self) -> Paint {
        let ga = Self::clamp01(self.state.global_alpha);
        match &self.state.fill_style {
            FillStyleKind::Color(c) => Paint::color(Self::apply_global_alpha(*c, ga)),
            FillStyleKind::LinearGradient {
                x0,
                y0,
                x1,
                y1,
                stops,
            } => Paint::linear_gradient_stops(
                *x0,
                *y0,
                *x1,
                *y1,
                stops
                    .iter()
                    .copied()
                    .map(|(offset, color)| (offset, Self::apply_global_alpha(color, ga))),
            ),
            FillStyleKind::RadialGradient {
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                stops,
            } => {
                let center =
                    if (*x0 - *x1).abs() <= f32::EPSILON && (*y0 - *y1).abs() <= f32::EPSILON {
                        (*x0, *y0)
                    } else {
                        (*x1, *y1)
                    };
                Paint::radial_gradient_stops(
                    center.0,
                    center.1,
                    (*r0).max(0.0),
                    (*r1).max(0.0),
                    stops
                        .iter()
                        .copied()
                        .map(|(offset, color)| (offset, Self::apply_global_alpha(color, ga))),
                )
            }
            FillStyleKind::ConicGradient { cx, cy, stops } => Paint::conic_gradient_stops(
                *cx,
                *cy,
                stops
                    .iter()
                    .copied()
                    .map(|(offset, color)| (offset, Self::apply_global_alpha(color, ga))),
            ),
            FillStyleKind::Pattern {
                image_id,
                width,
                height,
                ..
            } => Paint::image(*image_id, 0.0, 0.0, *width, *height, 0.0, ga),
        }
    }

    fn build_stroke_paint(&self) -> Paint {
        let ga = Self::clamp01(self.state.global_alpha);
        let base = match &self.state.stroke_style {
            FillStyleKind::Color(c) => Paint::color(Self::apply_global_alpha(*c, ga)),
            FillStyleKind::LinearGradient {
                x0,
                y0,
                x1,
                y1,
                stops,
            } => Paint::linear_gradient_stops(
                *x0,
                *y0,
                *x1,
                *y1,
                stops
                    .iter()
                    .copied()
                    .map(|(offset, color)| (offset, Self::apply_global_alpha(color, ga))),
            ),
            FillStyleKind::RadialGradient {
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                stops,
            } => {
                let center =
                    if (*x0 - *x1).abs() <= f32::EPSILON && (*y0 - *y1).abs() <= f32::EPSILON {
                        (*x0, *y0)
                    } else {
                        (*x1, *y1)
                    };
                Paint::radial_gradient_stops(
                    center.0,
                    center.1,
                    (*r0).max(0.0),
                    (*r1).max(0.0),
                    stops
                        .iter()
                        .copied()
                        .map(|(offset, color)| (offset, Self::apply_global_alpha(color, ga))),
                )
            }
            FillStyleKind::ConicGradient { cx, cy, stops } => Paint::conic_gradient_stops(
                *cx,
                *cy,
                stops
                    .iter()
                    .copied()
                    .map(|(offset, color)| (offset, Self::apply_global_alpha(color, ga))),
            ),
            FillStyleKind::Pattern {
                image_id,
                width,
                height,
                ..
            } => Paint::image(*image_id, 0.0, 0.0, *width, *height, 0.0, ga),
        };
        base.with_line_width(self.state.line_width)
            .with_line_cap(match self.state.line_cap {
                CanvasLineCap::Butt => LineCap::Butt,
                CanvasLineCap::Round => LineCap::Round,
                CanvasLineCap::Square => LineCap::Square,
            })
            .with_line_join(match self.state.line_join {
                CanvasLineJoin::Miter => LineJoin::Miter,
                CanvasLineJoin::Round => LineJoin::Round,
                CanvasLineJoin::Bevel => LineJoin::Bevel,
            })
            .with_miter_limit(self.state.miter_limit)
    }

    pub fn flush(&mut self) {
        self.canvas.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_conic_stops_applies_start_angle_shift() {
        let stops = vec![(0.0, Color::rgb(255, 0, 0)), (0.5, Color::rgb(0, 0, 255))];

        let rotated = Canvas2DContext::rotate_conic_stops(stops, std::f32::consts::FRAC_PI_2);

        assert!((rotated[0].0 - 0.25).abs() < 1e-6);
        assert!((rotated[1].0 - 0.75).abs() < 1e-6);
    }

    #[test]
    fn rotate_conic_stops_wraps_and_keeps_sorted_order() {
        let stops = vec![(0.8, Color::rgb(255, 0, 0)), (0.2, Color::rgb(0, 255, 0))];

        let rotated = Canvas2DContext::rotate_conic_stops(stops, std::f32::consts::PI);

        assert!((rotated[0].0 - 0.3).abs() < 1e-6);
        assert!((rotated[1].0 - 0.7).abs() < 1e-6);
    }
}
