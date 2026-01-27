use deno_core::{op2, OpState};
use femtovg::Color;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use tracing::error;

use shared::{
    op_state::CanvasOpState,
    protocol::{
        render_cmd::{Canvas2DCmd, DrawImageEntry, RenderCommand, TextAlign, TextBaseline, TextMetrics},
        send_render_with_resp_sync,
    },
};

const OP_CREATE_CTX2D: &str = "canvas2d create context";

#[inline]
fn send_2d(ctx: &CanvasOpState, canvas_id: u32, cmd: Canvas2DCmd) {
    if let Err(e) = ctx.tx.send(RenderCommand::Canvas2D { canvas_id, cmd }) {
        error!("send_2d failed: {e}");
    }
}

static NAMED_COLORS: Lazy<HashMap<&'static str, femtovg::Color>> = Lazy::new(|| {
    [
        ("aliceblue", Color::rgb(240, 248, 255)),
        ("antiquewhite", Color::rgb(250, 235, 215)),
        ("aqua", Color::rgb(0, 255, 255)),
        ("aquamarine", Color::rgb(127, 255, 212)),
        ("azure", Color::rgb(240, 255, 255)),
        ("beige", Color::rgb(245, 245, 220)),
        ("bisque", Color::rgb(255, 228, 196)),
        ("black", Color::rgb(0, 0, 0)),
        ("blanchedalmond", Color::rgb(255, 235, 205)),
        ("blue", Color::rgb(0, 0, 255)),
        ("blueviolet", Color::rgb(138, 43, 226)),
        ("brown", Color::rgb(165, 42, 42)),
        ("burlywood", Color::rgb(222, 184, 135)),
        ("cadetblue", Color::rgb(95, 158, 160)),
        ("chartreuse", Color::rgb(127, 255, 0)),
        ("chocolate", Color::rgb(210, 105, 30)),
        ("coral", Color::rgb(255, 127, 80)),
        ("cornflowerblue", Color::rgb(100, 149, 237)),
        ("cornsilk", Color::rgb(255, 248, 220)),
        ("crimson", Color::rgb(220, 20, 60)),
        ("cyan", Color::rgb(0, 255, 255)),
        ("darkblue", Color::rgb(0, 0, 139)),
        ("darkcyan", Color::rgb(0, 139, 139)),
        ("darkgoldenrod", Color::rgb(184, 134, 11)),
        ("darkgray", Color::rgb(169, 169, 169)),
        ("darkgreen", Color::rgb(0, 100, 0)),
        ("darkgrey", Color::rgb(169, 169, 169)),
        ("darkkhaki", Color::rgb(189, 183, 107)),
        ("darkmagenta", Color::rgb(139, 0, 139)),
        ("darkolivegreen", Color::rgb(85, 107, 47)),
        ("darkorange", Color::rgb(255, 140, 0)),
        ("darkorchid", Color::rgb(153, 50, 204)),
        ("darkred", Color::rgb(139, 0, 0)),
        ("darksalmon", Color::rgb(233, 150, 122)),
        ("darkseagreen", Color::rgb(143, 188, 143)),
        ("darkslateblue", Color::rgb(72, 61, 139)),
        ("darkslategray", Color::rgb(47, 79, 79)),
        ("darkslategrey", Color::rgb(47, 79, 79)),
        ("darkturquoise", Color::rgb(0, 206, 209)),
        ("darkviolet", Color::rgb(148, 0, 211)),
        ("deeppink", Color::rgb(255, 20, 147)),
        ("deepskyblue", Color::rgb(0, 191, 255)),
        ("dimgray", Color::rgb(105, 105, 105)),
        ("dimgrey", Color::rgb(105, 105, 105)),
        ("dodgerblue", Color::rgb(30, 144, 255)),
        ("firebrick", Color::rgb(178, 34, 34)),
        ("floralwhite", Color::rgb(255, 250, 240)),
        ("forestgreen", Color::rgb(34, 139, 34)),
        ("fuchsia", Color::rgb(255, 0, 255)),
        ("gainsboro", Color::rgb(220, 220, 220)),
        ("ghostwhite", Color::rgb(248, 248, 255)),
        ("gold", Color::rgb(255, 215, 0)),
        ("goldenrod", Color::rgb(218, 165, 32)),
        ("gray", Color::rgb(128, 128, 128)),
        ("green", Color::rgb(0, 128, 0)),
        ("greenyellow", Color::rgb(173, 255, 47)),
        ("grey", Color::rgb(128, 128, 128)),
        ("honeydew", Color::rgb(240, 255, 240)),
        ("hotpink", Color::rgb(255, 105, 180)),
        ("indianred", Color::rgb(205, 92, 92)),
        ("indigo", Color::rgb(75, 0, 130)),
        ("ivory", Color::rgb(255, 255, 240)),
        ("khaki", Color::rgb(240, 230, 140)),
        ("lavender", Color::rgb(230, 230, 250)),
        ("lavenderblush", Color::rgb(255, 240, 245)),
        ("lawngreen", Color::rgb(124, 252, 0)),
        ("lemonchiffon", Color::rgb(255, 250, 205)),
        ("lightblue", Color::rgb(173, 216, 230)),
        ("lightcoral", Color::rgb(240, 128, 128)),
        ("lightcyan", Color::rgb(224, 255, 255)),
        ("lightgoldenrodyellow", Color::rgb(250, 250, 210)),
        ("lightgray", Color::rgb(211, 211, 211)),
        ("lightgreen", Color::rgb(144, 238, 144)),
        ("lightgrey", Color::rgb(211, 211, 211)),
        ("lightpink", Color::rgb(255, 182, 193)),
        ("lightsalmon", Color::rgb(255, 160, 122)),
        ("lightseagreen", Color::rgb(32, 178, 170)),
        ("lightskyblue", Color::rgb(135, 206, 250)),
        ("lightslategray", Color::rgb(119, 136, 153)),
        ("lightslategrey", Color::rgb(119, 136, 153)),
        ("lightsteelblue", Color::rgb(176, 196, 222)),
        ("lightyellow", Color::rgb(255, 255, 224)),
        ("lime", Color::rgb(0, 255, 0)),
        ("limegreen", Color::rgb(50, 205, 50)),
        ("linen", Color::rgb(250, 240, 230)),
        ("magenta", Color::rgb(255, 0, 255)),
        ("maroon", Color::rgb(128, 0, 0)),
        ("mediumaquamarine", Color::rgb(102, 205, 170)),
        ("mediumblue", Color::rgb(0, 0, 205)),
        ("mediumorchid", Color::rgb(186, 85, 211)),
        ("mediumpurple", Color::rgb(147, 112, 219)),
        ("mediumseagreen", Color::rgb(60, 179, 113)),
        ("mediumslateblue", Color::rgb(123, 104, 238)),
        ("mediumspringgreen", Color::rgb(0, 250, 154)),
        ("mediumturquoise", Color::rgb(72, 209, 204)),
        ("mediumvioletred", Color::rgb(199, 21, 133)),
        ("midnightblue", Color::rgb(25, 25, 112)),
        ("mintcream", Color::rgb(245, 255, 250)),
        ("mistyrose", Color::rgb(255, 228, 225)),
        ("moccasin", Color::rgb(255, 228, 181)),
        ("navajowhite", Color::rgb(255, 222, 173)),
        ("navy", Color::rgb(0, 0, 128)),
        ("oldlace", Color::rgb(253, 245, 230)),
        ("olive", Color::rgb(128, 128, 0)),
        ("olivedrab", Color::rgb(107, 142, 35)),
        ("orange", Color::rgb(255, 165, 0)),
        ("orangered", Color::rgb(255, 69, 0)),
        ("orchid", Color::rgb(218, 112, 214)),
        ("palegoldenrod", Color::rgb(238, 232, 170)),
        ("palegreen", Color::rgb(152, 251, 152)),
        ("paleturquoise", Color::rgb(175, 238, 238)),
        ("palevioletred", Color::rgb(219, 112, 147)),
        ("papayawhip", Color::rgb(255, 239, 213)),
        ("peachpuff", Color::rgb(255, 218, 185)),
        ("peru", Color::rgb(205, 133, 63)),
        ("pink", Color::rgb(255, 192, 203)),
        ("plum", Color::rgb(221, 160, 221)),
        ("powderblue", Color::rgb(176, 224, 230)),
        ("purple", Color::rgb(128, 0, 128)),
        ("rebeccapurple", Color::rgb(102, 51, 153)),
        ("red", Color::rgb(255, 0, 0)),
        ("rosybrown", Color::rgb(188, 143, 143)),
        ("royalblue", Color::rgb(65, 105, 225)),
        ("saddlebrown", Color::rgb(139, 69, 19)),
        ("salmon", Color::rgb(250, 128, 114)),
        ("sandybrown", Color::rgb(244, 164, 96)),
        ("seagreen", Color::rgb(46, 139, 87)),
        ("seashell", Color::rgb(255, 245, 238)),
        ("sienna", Color::rgb(160, 82, 45)),
        ("silver", Color::rgb(192, 192, 192)),
        ("skyblue", Color::rgb(135, 206, 235)),
        ("slateblue", Color::rgb(106, 90, 205)),
        ("slategray", Color::rgb(112, 128, 144)),
        ("slategrey", Color::rgb(112, 128, 144)),
        ("snow", Color::rgb(255, 250, 250)),
        ("springgreen", Color::rgb(0, 255, 127)),
        ("steelblue", Color::rgb(70, 130, 180)),
        ("tan", Color::rgb(210, 180, 140)),
        ("teal", Color::rgb(0, 128, 128)),
        ("thistle", Color::rgb(216, 191, 216)),
        ("tomato", Color::rgb(255, 99, 71)),
        ("turquoise", Color::rgb(64, 224, 208)),
        ("violet", Color::rgb(238, 130, 238)),
        ("wheat", Color::rgb(245, 222, 179)),
        ("white", Color::rgb(255, 255, 255)),
        ("whitesmoke", Color::rgb(245, 245, 245)),
        ("yellow", Color::rgb(255, 255, 0)),
        ("yellowgreen", Color::rgb(154, 205, 50)),
    ]
    .into_iter()
    .collect()
});

fn parse_color_string(s: &str) -> femtovg::Color {
    let s = s.trim().to_lowercase();
    if s.starts_with('#') { return Color::hex(&s); }
    if let Some(inner) = s.strip_prefix("rgba(").and_then(|v| v.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').map(|x| x.trim()).collect();
        if parts.len() == 4 {
            let r = parts[0].parse::<u8>().unwrap_or(0);
            let g = parts[1].parse::<u8>().unwrap_or(0);
            let b = parts[2].parse::<u8>().unwrap_or(0);
            let a = (parts[3].parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0) * 255.0) as u8;
            return Color::rgba(r, g, b, a);
        }
        return Color::black();
    }
    if let Some(inner) = s.strip_prefix("rgb(").and_then(|v| v.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').map(|x| x.trim()).collect();
        if parts.len() == 3 {
            let r = parts[0].parse::<u8>().unwrap_or(0);
            let g = parts[1].parse::<u8>().unwrap_or(0);
            let b = parts[2].parse::<u8>().unwrap_or(0);
            return Color::rgb(r, g, b);
        }
        return Color::black();
    }
    NAMED_COLORS.get(s.as_str()).copied().unwrap_or(Color::black())
}

#[op2(fast)]
pub fn op_create_context_2d(state: &mut OpState, #[smi] canvas_id: u32) -> i32 {
    let ctx = state.borrow::<CanvasOpState>();
    match send_render_with_resp_sync(ctx, OP_CREATE_CTX2D, |resp| RenderCommand::Canvas2D { canvas_id, cmd: Canvas2DCmd::CreateContext2D { resp } }) {
        Ok(id) => id as i32,
        Err(e) => { error!("{OP_CREATE_CTX2D} failed: {e}"); -1 }
    }
}

// Path methods
#[op2(fast)] pub fn op_begin_path(state: &mut OpState, #[smi] canvas_id: u32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::BeginPath); }
#[op2(fast)] pub fn op_close_path(state: &mut OpState, #[smi] canvas_id: u32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::ClosePath); }
#[op2(fast)] pub fn op_move_to(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::MoveTo { x, y }); }
#[op2(fast)] pub fn op_line_to(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::LineTo { x, y }); }
#[op2(fast)] pub fn op_quadratic_curve_to(state: &mut OpState, #[smi] canvas_id: u32, cpx: f32, cpy: f32, x: f32, y: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::QuadraticCurveTo { cpx, cpy, x, y }); }
#[op2(fast)] pub fn op_bezier_curve_to(state: &mut OpState, #[smi] canvas_id: u32, cp1x: f32, cp1y: f32, cp2x: f32, cp2y: f32, x: f32, y: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::BezierCurveTo { cp1x, cp1y, cp2x, cp2y, x, y }); }
#[op2(fast)] pub fn op_arc(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32, radius: f32, start_angle: f32, end_angle: f32, counterclockwise: bool) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::Arc { x, y, radius, start_angle, end_angle, counterclockwise }); }
#[op2(fast)] pub fn op_arc_to(state: &mut OpState, #[smi] canvas_id: u32, x1: f32, y1: f32, x2: f32, y2: f32, radius: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::ArcTo { x1, y1, x2, y2, radius }); }
#[op2(fast)] pub fn op_rect(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32, w: f32, h: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::Rect { x, y, w, h }); }
#[op2(fast)] #[allow(clippy::too_many_arguments)] pub fn op_ellipse(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32, radius_x: f32, radius_y: f32, rotation: f32, start_angle: f32, end_angle: f32, counterclockwise: bool) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::Ellipse { x, y, radius_x, radius_y, rotation, start_angle, end_angle, counterclockwise }); }

// Drawing methods
#[op2(fast)] pub fn op_fill(state: &mut OpState, #[smi] canvas_id: u32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::Fill); }
#[op2(fast)] pub fn op_stroke(state: &mut OpState, #[smi] canvas_id: u32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::Stroke); }
#[op2(fast)] pub fn op_clip(state: &mut OpState, #[smi] canvas_id: u32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::Clip); }

// Rectangle methods
#[op2(fast)] pub fn op_fill_rect(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32, w: f32, h: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::FillRect { x, y, w, h }); }
#[op2(fast)] pub fn op_stroke_rect(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32, w: f32, h: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::StrokeRect { x, y, w, h }); }
#[op2(fast)] pub fn op_clear_rect(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32, w: f32, h: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::ClearRect { x, y, w, h }); }

// Text methods
#[op2(fast)] pub fn op_fill_text(state: &mut OpState, #[smi] canvas_id: u32, #[string] text: String, x: f32, y: f32, max_width: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::FillText { text, x, y, max_width }); }
#[op2(fast)] pub fn op_stroke_text(state: &mut OpState, #[smi] canvas_id: u32, #[string] text: String, x: f32, y: f32, max_width: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::StrokeText { text, x, y, max_width }); }

// Style setters
#[op2(fast)] pub fn op_set_fill_style(state: &mut OpState, #[smi] canvas_id: u32, #[string] color_str: String) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::SetFillStyle { color: parse_color_string(&color_str) }); }
#[op2(fast)] pub fn op_set_stroke_style(state: &mut OpState, #[smi] canvas_id: u32, #[string] color_str: String) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::SetStrokeStyle { color: parse_color_string(&color_str) }); }
#[op2(fast)] pub fn op_set_line_width(state: &mut OpState, #[smi] canvas_id: u32, width: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::SetLineWidth { width }); }
#[op2(fast)] pub fn op_set_line_cap(state: &mut OpState, #[smi] canvas_id: u32, #[smi] cap: u8) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::SetLineCap { cap }); }
#[op2(fast)] pub fn op_set_line_join(state: &mut OpState, #[smi] canvas_id: u32, #[smi] join: u8) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::SetLineJoin { join }); }
#[op2(fast)] pub fn op_set_miter_limit(state: &mut OpState, #[smi] canvas_id: u32, limit: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::SetMiterLimit { limit }); }
#[op2(fast)] pub fn op_set_global_alpha(state: &mut OpState, #[smi] canvas_id: u32, alpha: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::SetGlobalAlpha { alpha }); }
#[op2(fast)] pub fn op_set_font(state: &mut OpState, #[smi] canvas_id: u32, #[string] font: String) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::SetFont { font }); }
#[op2(fast)] pub fn op_set_text_align(state: &mut OpState, #[smi] canvas_id: u32, #[smi] align: u8) {
    let align = match align { 0 => TextAlign::Start, 1 => TextAlign::End, 2 => TextAlign::Left, 3 => TextAlign::Right, 4 => TextAlign::Center, _ => TextAlign::Start };
    send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::SetTextAlign { align });
}
#[op2(fast)] pub fn op_set_text_baseline(state: &mut OpState, #[smi] canvas_id: u32, #[smi] baseline: u8) {
    let baseline = match baseline { 0 => TextBaseline::Top, 1 => TextBaseline::Hanging, 2 => TextBaseline::Middle, 3 => TextBaseline::Alphabetic, 4 => TextBaseline::Ideographic, 5 => TextBaseline::Bottom, _ => TextBaseline::Alphabetic };
    send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::SetTextBaseline { baseline });
}

// State methods
#[op2(fast)] pub fn op_save(state: &mut OpState, #[smi] canvas_id: u32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::Save); }
#[op2(fast)] pub fn op_restore(state: &mut OpState, #[smi] canvas_id: u32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::Restore); }

// Transform methods
#[op2(fast)] pub fn op_translate(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::Translate { x, y }); }
#[op2(fast)] pub fn op_rotate(state: &mut OpState, #[smi] canvas_id: u32, angle: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::Rotate { angle }); }
#[op2(fast)] pub fn op_scale(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::Scale { x, y }); }
#[op2(fast)] #[allow(clippy::too_many_arguments)] pub fn op_set_transform(state: &mut OpState, #[smi] canvas_id: u32, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::SetTransform { a, b, c, d, e, f }); }
#[op2(fast)] pub fn op_reset_transform(state: &mut OpState, #[smi] canvas_id: u32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::ResetTransform); }

// Image methods
#[op2(fast)] #[allow(clippy::too_many_arguments)] pub fn op_draw_image(state: &mut OpState, #[smi] canvas_id: u32, #[smi] image_id: u32, sx: f32, sy: f32, sw: f32, sh: f32, dx: f32, dy: f32, dw: f32, dh: f32) { send_2d(state.borrow::<CanvasOpState>(), canvas_id, Canvas2DCmd::DrawImage { image_id, sx, sy, sw, sh, dx, dy, dw, dh }); }

// Measurement methods (synchronous)
const OP_MEASURE_TEXT: &str = "canvas2d measure_text";
#[op2] #[serde] pub fn op_measure_text(state: &mut OpState, #[smi] canvas_id: u32, #[string] text: String) -> TextMetrics {
    let ctx = state.borrow::<CanvasOpState>();
    match send_render_with_resp_sync(ctx, OP_MEASURE_TEXT, |resp| RenderCommand::Canvas2D { canvas_id, cmd: Canvas2DCmd::MeasureText { text, resp } }) {
        Ok(m) => m,
        Err(e) => { error!("{OP_MEASURE_TEXT} failed: {e}"); TextMetrics { width: 0.0, actual_bounding_box_left: 0.0, actual_bounding_box_right: 0.0, actual_bounding_box_ascent: 0.0, actual_bounding_box_descent: 0.0, font_bounding_box_ascent: 0.0, font_bounding_box_descent: 0.0 } }
    }
}

const OP_GET_IMAGE_DATA: &str = "canvas2d get_image_data";
#[op2] #[buffer] pub fn op_get_image_data(state: &mut OpState, #[smi] canvas_id: u32, x: i32, y: i32, width: u32, height: u32) -> Vec<u8> {
    let ctx = state.borrow::<CanvasOpState>();
    match send_render_with_resp_sync(ctx, OP_GET_IMAGE_DATA, |resp| RenderCommand::Canvas2D { canvas_id, cmd: Canvas2DCmd::GetImageData { x, y, width, height, resp } }) {
        Ok(d) => d,
        Err(e) => { error!("{OP_GET_IMAGE_DATA} failed: {e}"); vec![] }
    }
}

/// Batch draw images for better performance
/// Input format: [[image_id, sx, sy, sw, sh, dx, dy, dw, dh], ...]
#[op2(fast)]
pub fn op_draw_image_batch(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[buffer] data: &[u8],
) {
    // Parse the buffer: each entry is 9 f32s (36 bytes)
    // image_id (as f32), sx, sy, sw, sh, dx, dy, dw, dh
    const ENTRY_SIZE: usize = 9 * 4; // 9 floats * 4 bytes

    if data.len() % ENTRY_SIZE != 0 {
        error!("op_draw_image_batch: invalid buffer size");
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

    send_2d(
        state.borrow::<CanvasOpState>(),
        canvas_id,
        Canvas2DCmd::DrawImageBatch { draws },
    );
}
