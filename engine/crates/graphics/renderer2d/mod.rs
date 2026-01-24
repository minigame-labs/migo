#![allow(dead_code)]

use femtovg::{
    renderer::OpenGl, Canvas as FvCanvas, Color, FontId, ImageId, LineCap, LineJoin, Paint, Path,
    Transform2D,
};

mod font;
mod handler;

pub(crate) use font::*;
pub(crate) use handler::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextAlign {
    Start,
    End,
    Left,
    Right,
    Center,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextBaseline {
    Top,
    Hanging,
    Middle,
    Alphabetic,
    Ideographic,
    Bottom,
}

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
    pub clip_rect: Option<(f32, f32, f32, f32)>, // (x, y, w, h)
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
            text_align: TextAlign::Start,
            text_baseline: TextBaseline::Alphabetic,

            transform: Transform2D::identity(),
            clip_rect: None,
        }
    }
}

pub(crate) struct Canvas2DContext {
    pub canvas: FvCanvas<OpenGl>,
    pub font_manager: FontManager,

    pub state: Canvas2DState,
    pub stack: Vec<Canvas2DState>,
    pub current_path: Path,
}

impl Canvas2DContext {
    pub(crate) fn new(canvas: FvCanvas<OpenGl>, font_manager: FontManager) -> Self {
        Self {
            canvas,
            font_manager,
            state: Canvas2DState::default(),
            stack: Vec::new(),
            current_path: Path::new(),
        }
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

    pub fn begin_path(&mut self) {
        self.current_path = Path::new();
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        self.current_path.move_to(x, y);
    }

    pub fn line_to(&mut self, x: f32, y: f32) {
        self.current_path.line_to(x, y);
    }

    pub fn close_path(&mut self) {
        self.current_path.close();
    }


    pub fn set_fill_style_color(&mut self, color: Color) {
        self.state.fill_style = FillStyleKind::Color(color);
    }

    pub fn set_stroke_style_color(&mut self, color: Color) {
        self.state.stroke_style = color;
    }

    pub fn set_line_width(&mut self, w: f32) {
        self.state.line_width = w.max(0.0);
    }

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

    // ---------------- rect ops ----------------

    pub fn clear_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.canvas.save_with(|c| {
            c.reset_transform();
            // Use transparent black
            c.clear_rect(
                x.max(0.0) as u32,
                y.max(0.0) as u32,
                w.max(0.0) as u32,
                h.max(0.0) as u32,
                Color::rgba(0, 0, 0, 0),
            );
        });
    }

    pub fn fill(&mut self) {
        let paint = self.build_fill_paint();
        self.canvas.fill_path(&self.current_path, &paint);
    }

    pub fn stroke(&mut self) {
        let stroke = Self::apply_global_alpha(self.state.stroke_style, self.state.global_alpha);

        let paint = Paint::color(stroke)
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
            .with_miter_limit(self.state.miter_limit);

        self.canvas.stroke_path(&self.current_path, &paint);
    }

    pub fn set_transform(&mut self, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) {
        self.state.transform = Transform2D::new(a, b, c, d, e, f);
        self.canvas.set_transform(&self.state.transform);
    }

    pub fn reset_transform(&mut self) {
        self.state.transform = Transform2D::identity();
        self.canvas.reset_transform();
    }

    pub fn fill_text(&mut self, text: &str, x: f32, y: f32) {
        // Use fill paint color/gradient/pattern, but ensure globalAlpha is applied.
        let paint = self.build_fill_paint();
        let _ = self.canvas.fill_text(x, y, text, &paint);
    }

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

        let mut sx = sx;
        let mut sy = sy;
        let mut sw = sw;
        let mut sh = sh;

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

        let mut dw = dw;
        let mut dh = dh;

        if dw <= 0.0 {
            dw = sw;
        }
        if dh <= 0.0 {
            dh = sh;
        }

        if dw <= 0.0 || dh <= 0.0 {
            return;
        }

        let scale_x = dw / sw;
        let scale_y = dh / sh;

        let source_is_whole = sx == 0.0
            && sy == 0.0
            && (sw - img_w).abs() < f32::EPSILON
            && (sh - img_h).abs() < f32::EPSILON;

        let ga = Self::clamp01(self.state.global_alpha);

        if source_is_whole {
            let anti_alias = (scale_x != 1.0) || (scale_y != 1.0);
            let paint = Paint::image(image_id, dx, dy, dw, dh, 0.0, ga).with_anti_alias(anti_alias);

            let mut path = Path::new();
            path.rect(dx, dy, dw, dh);
            self.canvas.save_with(|canvas| {
                canvas.fill_path(&path, &paint);
            });
            return;
        }

        // For sub-rect: draw full image scaled so that requested source rect maps to destination rect,
        // then clip to destination rect.
        let draw_x = dx - sx * scale_x;
        let draw_y = dy - sy * scale_y;
        let draw_w = img_w * scale_x;
        let draw_h = img_h * scale_y;

        let anti_alias = (scale_x != 1.0) || (scale_y != 1.0);
        let paint = Paint::image(image_id, draw_x, draw_y, draw_w, draw_h, 0.0, ga).with_anti_alias(anti_alias);

        let mut path = Path::new();
        path.rect(dx, dy, dw, dh);
        self.canvas.save_with(|canvas| {
            canvas.fill_path(&path, &paint);
        });
    }

    fn build_fill_paint(&self) -> Paint {
        let ga = Self::clamp01(self.state.global_alpha);

        match &self.state.fill_style {
            FillStyleKind::Color(c) => Paint::color(Self::apply_global_alpha(*c, ga)),

            FillStyleKind::Gradient {
                x0,
                y0,
                x1,
                y1,
                start_color,
                end_color,
            } => {
                let sc = Self::apply_global_alpha(*start_color, ga);
                let ec = Self::apply_global_alpha(*end_color, ga);
                Paint::linear_gradient(*x0, *y0, *x1, *y1, sc, ec)
            }

            FillStyleKind::Pattern { image_id, .. } => {
                Paint::image(*image_id, 0.0, 0.0, 1.0, 1.0, 0.0, ga)
            }
        }
    }

    pub fn flush(&mut self) {
        self.canvas.flush();
    }
}
