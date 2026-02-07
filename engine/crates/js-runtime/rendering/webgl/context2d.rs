//! Canvas 2D Context - Command Batching Implementation
//!
//! All draw commands within a RAF frame are batched via [`FrameCommandCollector`]
//! and sent as a single `Canvas2DBatch` message to the render thread, reducing
//! IPC overhead to one message per canvas per frame.
//!
//! Sync operations (`op_create_context_2d`, `op_measure_text`, `op_get_image_data`)
//! use `RenderCommand::Canvas2D` for synchronous request/response.

use deno_core::{op2, OpState};
use femtovg::Color;
use once_cell::sync::Lazy;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use tracing::{error, trace};

use shared::{
    op_state::CanvasOpState,
    protocol::{
        render_cmd::{Canvas2DCmd, RenderCommand, TextAlign, TextBaseline, TextMetrics},
        send_render_with_resp_sync,
    },
};

// ============================================================================
// Color parsing
// ============================================================================

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

/// Case-insensitive prefix check without allocation.
#[inline]
fn starts_with_ci(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len()
        && s.as_bytes()[..prefix.len()]
            .iter()
            .zip(prefix.as_bytes())
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
}

/// Parse comma-separated values from an already-trimmed inner string.
/// Uses a stack-based array to avoid Vec allocation.
#[inline]
fn split_comma_parts(s: &str) -> ([&str; 4], usize) {
    let mut parts = [""; 4];
    let mut count = 0;
    for part in s.split(',') {
        if count >= 4 {
            return (parts, count + 1); // signal overflow
        }
        parts[count] = part.trim();
        count += 1;
    }
    (parts, count)
}

fn parse_color_string(s: &str) -> femtovg::Color {
    let s = s.trim();

    // #hex — no lowercase needed
    if s.starts_with('#') {
        return Color::hex(s);
    }

    // rgba(...) — case-insensitive prefix, zero-alloc
    if starts_with_ci(s, "rgba(") && s.ends_with(')') {
        let inner = &s[5..s.len() - 1];
        let (parts, count) = split_comma_parts(inner);
        if count == 4 {
            let r = parts[0].parse::<u8>().unwrap_or(0);
            let g = parts[1].parse::<u8>().unwrap_or(0);
            let b = parts[2].parse::<u8>().unwrap_or(0);
            let a = (parts[3].parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0) * 255.0) as u8;
            return Color::rgba(r, g, b, a);
        }
        return Color::black();
    }

    // rgb(...) — case-insensitive prefix, zero-alloc
    if starts_with_ci(s, "rgb(") && s.ends_with(')') {
        let inner = &s[4..s.len() - 1];
        let (parts, count) = split_comma_parts(inner);
        if count == 3 {
            let r = parts[0].parse::<u8>().unwrap_or(0);
            let g = parts[1].parse::<u8>().unwrap_or(0);
            let b = parts[2].parse::<u8>().unwrap_or(0);
            return Color::rgb(r, g, b);
        }
        return Color::black();
    }

    // Named colors — lowercase on stack buffer (max 24 bytes, no heap alloc)
    let bytes = s.as_bytes();
    if bytes.len() <= 24 {
        let mut buf = [0u8; 24];
        for (i, &b) in bytes.iter().enumerate() {
            buf[i] = b.to_ascii_lowercase();
        }
        // SAFETY: input is valid UTF-8, lowercasing ASCII preserves UTF-8 validity
        let lower = unsafe { std::str::from_utf8_unchecked(&buf[..bytes.len()]) };
        NAMED_COLORS.get(lower).copied().unwrap_or(Color::black())
    } else {
        Color::black()
    }
}

// ============================================================================
// Frame command collector (batches draw commands per frame)
// ============================================================================

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

    fn set_fill_color(&mut self, color: Color) {
        if self.fill_color != Some(color) {
            self.fill_color = Some(color);
            self.commands.push(Canvas2DCmd::SetFillStyle { color });
        }
    }

    fn set_stroke_color(&mut self, color: Color) {
        if self.stroke_color != Some(color) {
            self.stroke_color = Some(color);
            self.commands.push(Canvas2DCmd::SetStrokeStyle { color });
        }
    }

    fn set_line_width(&mut self, width: f32) {
        if self.line_width != Some(width) {
            self.line_width = Some(width);
            self.commands.push(Canvas2DCmd::SetLineWidth { width });
        }
    }

    fn set_global_alpha(&mut self, alpha: f32) {
        if self.global_alpha != Some(alpha) {
            self.global_alpha = Some(alpha);
            self.commands.push(Canvas2DCmd::SetGlobalAlpha { alpha });
        }
    }
}

/// Frame command collector stored in OpState.
///
/// Manages command buffers for multiple canvases within a single frame.
/// Optimized for the common single-canvas case: the first canvas used is
/// stored directly in `primary` (single `u32` comparison, no HashMap).
/// Additional canvases fall back to the `extra` HashMap.
pub struct FrameCommandCollector {
    /// Canvas ID occupying the primary slot (valid when `primary_active` is true).
    primary_id: Cell<u32>,
    /// Whether the primary slot has been claimed.
    primary_active: Cell<bool>,
    /// Fast-path buffer for the primary (usually only) canvas.
    primary: RefCell<FrameBuffer>,
    /// Overflow buffers for additional canvases (rare in practice).
    extra: RefCell<HashMap<u32, FrameBuffer>>,
    total_commands: u64,
    total_frames: u64,
}

impl FrameCommandCollector {
    pub fn new() -> Self {
        Self {
            primary_id: Cell::new(0),
            primary_active: Cell::new(false),
            primary: RefCell::new(FrameBuffer::new()),
            extra: RefCell::new(HashMap::new()),
            total_commands: 0,
            total_frames: 0,
        }
    }

    /// Execute a closure with the mutable buffer for `canvas_id`, creating it if needed.
    /// Fast path: single u32 comparison for the primary canvas (no hash, no HashMap lookup).
    #[inline]
    fn with_buffer<R>(&self, canvas_id: u32, f: impl FnOnce(&mut FrameBuffer) -> R) -> R {
        if self.primary_active.get() {
            if self.primary_id.get() == canvas_id {
                return f(&mut self.primary.borrow_mut());
            }
        } else {
            self.primary_id.set(canvas_id);
            self.primary_active.set(true);
            return f(&mut self.primary.borrow_mut());
        }
        // Slow path: multi-canvas fallback
        let mut extra = self.extra.borrow_mut();
        let buf = extra.entry(canvas_id).or_insert_with(FrameBuffer::new);
        f(buf)
    }

    pub fn frame_begin(&self, canvas_id: u32) {
        self.with_buffer(canvas_id, |buf| buf.clear());
    }

    pub fn frame_end(&mut self, canvas_id: u32) -> Option<Vec<Canvas2DCmd>> {
        // &mut self: use get_mut() on RefCells (zero-cost, no runtime borrow check).
        let buf = if self.primary_active.get() && self.primary_id.get() == canvas_id {
            self.primary.get_mut()
        } else {
            self.extra.get_mut().get_mut(&canvas_id)?
        };
        if buf.commands.is_empty() {
            return None;
        }
        self.total_commands += buf.commands.len() as u64;
        self.total_frames += 1;
        let commands = std::mem::take(&mut buf.commands);
        buf.clear();
        Some(commands)
    }

    /// Drain all active canvas buffers in a single pass. Returns (canvas_id, commands) pairs.
    /// Uses get_mut() throughout (zero-cost RefCell access via &mut self).
    pub fn frame_end_all(&mut self) -> Vec<(u32, Vec<Canvas2DCmd>)> {
        let mut results = Vec::new();
        if self.primary_active.get() {
            let canvas_id = self.primary_id.get();
            let buf = self.primary.get_mut();
            if !buf.commands.is_empty() {
                self.total_commands += buf.commands.len() as u64;
                self.total_frames += 1;
                results.push((canvas_id, std::mem::take(&mut buf.commands)));
                buf.clear();
            }
        }
        for (&canvas_id, buf) in self.extra.get_mut().iter_mut() {
            if !buf.commands.is_empty() {
                self.total_commands += buf.commands.len() as u64;
                self.total_frames += 1;
                results.push((canvas_id, std::mem::take(&mut buf.commands)));
                buf.clear();
            }
        }
        results
    }

    /// Reset dedup state and push Restore for an existing buffer (no-create).
    /// Uses get_mut() throughout (zero-cost RefCell access via &mut self).
    pub fn restore(&mut self, canvas_id: u32) {
        let buf = if self.primary_active.get() && self.primary_id.get() == canvas_id {
            Some(self.primary.get_mut())
        } else {
            self.extra.get_mut().get_mut(&canvas_id)
        };
        if let Some(buf) = buf {
            buf.fill_color = None;
            buf.stroke_color = None;
            buf.line_width = None;
            buf.global_alpha = None;
            buf.commands.push(Canvas2DCmd::Restore);
        }
    }

    #[inline]
    fn push(&self, canvas_id: u32, cmd: Canvas2DCmd) {
        self.with_buffer(canvas_id, |buf| buf.push(cmd));
    }

    #[inline]
    fn set_fill_color(&self, canvas_id: u32, color: Color) {
        self.with_buffer(canvas_id, |buf| buf.set_fill_color(color));
    }

    #[inline]
    fn set_stroke_color(&self, canvas_id: u32, color: Color) {
        self.with_buffer(canvas_id, |buf| buf.set_stroke_color(color));
    }

    #[inline]
    fn set_line_width(&self, canvas_id: u32, width: f32) {
        self.with_buffer(canvas_id, |buf| buf.set_line_width(width));
    }

    #[inline]
    fn set_global_alpha(&self, canvas_id: u32, alpha: f32) {
        self.with_buffer(canvas_id, |buf| buf.set_global_alpha(alpha));
    }
}

impl Default for FrameCommandCollector {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Sync operations (request/response via RenderCommand::Canvas2D)
// ============================================================================

const OP_CREATE_CTX2D: &str = "canvas2d create context";

#[op2(fast)]
pub fn op_create_context_2d(state: &mut OpState, #[smi] canvas_id: u32) -> i32 {
    let ctx = state.borrow::<CanvasOpState>();
    match send_render_with_resp_sync(ctx, OP_CREATE_CTX2D, |resp| RenderCommand::Canvas2D { canvas_id, cmd: Canvas2DCmd::CreateContext2D { resp } }) {
        Ok(id) => id as i32,
        Err(e) => { error!("{OP_CREATE_CTX2D} failed: {e}"); -1 }
    }
}

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

// ============================================================================
// Frame lifecycle operations
// ============================================================================

#[op2(fast)]
pub fn op_frame_begin(state: &mut OpState, #[smi] canvas_id: u32) {
    if let Some(collector) = state.try_borrow_mut::<FrameCommandCollector>() {
        collector.frame_begin(canvas_id);
    }
}

#[op2(fast)]
pub fn op_frame_end(state: &mut OpState, #[smi] canvas_id: u32) {
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
            trace!("op_frame_end: sending {} commands for canvas {}", cmds.len(), canvas_id);
            if let Err(e) = ctx.tx.send(RenderCommand::Canvas2DBatch {
                canvas_id,
                commands: cmds
            }) {
                error!("op_frame_end: send failed: {e}");
            }
        }
    }
}

#[op2(fast)]
pub fn op_frame_end_all(state: &mut OpState) {
    let pairs = {
        if let Some(collector) = state.try_borrow_mut::<FrameCommandCollector>() {
            collector.frame_end_all()
        } else {
            return;
        }
    };

    for (canvas_id, cmds) in pairs {
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

#[op2(fast)]
pub fn op_invalidate(state: &mut OpState, #[smi] _canvas_id: u32) {
    let ctx = state.borrow::<CanvasOpState>();
    if let Err(e) = ctx.tx.send(RenderCommand::Invalidate) {
        error!("op_invalidate: send failed: {e}");
    }
}

// ============================================================================
// Batched Canvas 2D operations
// ============================================================================

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
batched_op!(op_begin_path, Canvas2DCmd::BeginPath);
batched_op!(op_close_path, Canvas2DCmd::ClosePath);
batched_op!(op_fill, Canvas2DCmd::Fill);
batched_op!(op_stroke, Canvas2DCmd::Stroke);
batched_op!(op_clip, Canvas2DCmd::Clip);

// State operations
batched_op!(op_save, Canvas2DCmd::Save);

#[op2(fast)]
pub fn op_restore(state: &mut OpState, #[smi] canvas_id: u32) {
    if let Some(collector) = state.try_borrow_mut::<FrameCommandCollector>() {
        collector.restore(canvas_id);
    }
}

// Transform operations
batched_op!(op_reset_transform, Canvas2DCmd::ResetTransform);

#[op2(fast)]
pub fn op_move_to(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::MoveTo { x, y });
    }
}

#[op2(fast)]
pub fn op_line_to(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::LineTo { x, y });
    }
}

#[op2(fast)]
pub fn op_quadratic_curve_to(
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
pub fn op_bezier_curve_to(
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
pub fn op_arc(
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
pub fn op_arc_to(
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
pub fn op_rect(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32, w: f32, h: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::Rect { x, y, w, h });
    }
}

#[op2(fast)]
#[allow(clippy::too_many_arguments)]
pub fn op_ellipse(
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
pub fn op_fill_rect(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32, w: f32, h: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::FillRect { x, y, w, h });
    }
}

#[op2(fast)]
pub fn op_stroke_rect(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32, w: f32, h: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::StrokeRect { x, y, w, h });
    }
}

#[op2(fast)]
pub fn op_clear_rect(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32, w: f32, h: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::ClearRect { x, y, w, h });
    }
}

// Text operations
#[op2(fast)]
pub fn op_fill_text(
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
pub fn op_stroke_text(
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
pub fn op_set_fill_style(state: &mut OpState, #[smi] canvas_id: u32, #[string] color_str: String) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        let color = parse_color_string(&color_str);
        collector.set_fill_color(canvas_id, color);
    }
}

#[op2(fast)]
pub fn op_set_stroke_style(state: &mut OpState, #[smi] canvas_id: u32, #[string] color_str: String) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        let color = parse_color_string(&color_str);
        collector.set_stroke_color(canvas_id, color);
    }
}

#[op2(fast)]
pub fn op_set_line_width(state: &mut OpState, #[smi] canvas_id: u32, width: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.set_line_width(canvas_id, width);
    }
}

#[op2(fast)]
pub fn op_set_line_cap(state: &mut OpState, #[smi] canvas_id: u32, #[smi] cap: u8) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::SetLineCap { cap });
    }
}

#[op2(fast)]
pub fn op_set_line_join(state: &mut OpState, #[smi] canvas_id: u32, #[smi] join: u8) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::SetLineJoin { join });
    }
}

#[op2(fast)]
pub fn op_set_miter_limit(state: &mut OpState, #[smi] canvas_id: u32, limit: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::SetMiterLimit { limit });
    }
}

#[op2(fast)]
pub fn op_set_global_alpha(state: &mut OpState, #[smi] canvas_id: u32, alpha: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.set_global_alpha(canvas_id, alpha);
    }
}

#[op2(fast)]
pub fn op_set_font(state: &mut OpState, #[smi] canvas_id: u32, #[string] font: String) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::SetFont { font });
    }
}

#[op2(fast)]
pub fn op_set_text_align(state: &mut OpState, #[smi] canvas_id: u32, #[smi] align: u8) {
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
pub fn op_set_text_baseline(state: &mut OpState, #[smi] canvas_id: u32, #[smi] baseline: u8) {
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
pub fn op_translate(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::Translate { x, y });
    }
}

#[op2(fast)]
pub fn op_rotate(state: &mut OpState, #[smi] canvas_id: u32, angle: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::Rotate { angle });
    }
}

#[op2(fast)]
pub fn op_scale(state: &mut OpState, #[smi] canvas_id: u32, x: f32, y: f32) {
    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::Scale { x, y });
    }
}

#[op2(fast)]
#[allow(clippy::too_many_arguments)]
pub fn op_set_transform(
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
pub fn op_draw_image(
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

#[op2(fast)]
pub fn op_draw_image_batch(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[buffer] data: &[u8],
) {
    use shared::protocol::render_cmd::DrawImageEntry;

    const ENTRY_SIZE: usize = 9 * 4;

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

    if let Some(collector) = state.try_borrow::<FrameCommandCollector>() {
        collector.push(canvas_id, Canvas2DCmd::DrawImageBatch { draws });
    }
}
