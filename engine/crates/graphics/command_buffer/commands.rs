//! Render command definitions
//!
//! These are compact, cache-friendly command structures optimized for
//! batch processing. Unlike the protocol commands, these are designed
//! for efficient storage and iteration.

use femtovg::Color;

/// Canvas 2D rendering command
///
/// Each variant is designed to be as compact as possible while
/// containing all necessary data for execution.
#[derive(Debug, Clone)]
pub enum Canvas2DCommand {
    // ========== State Commands ==========
    /// Save the current drawing state
    Save,
    /// Restore the previously saved state
    Restore,
    
    // ========== Style Commands ==========
    /// Set fill style to a solid color
    SetFillColor(Color),
    /// Set stroke style to a solid color
    SetStrokeColor(Color),
    /// Set fill style to a linear gradient
    SetFillGradient {
        x0: f32, y0: f32,
        x1: f32, y1: f32,
        start: Color,
        end: Color,
    },
    /// Set line width
    SetLineWidth(f32),
    /// Set line cap style (0=butt, 1=round, 2=square)
    SetLineCap(u8),
    /// Set line join style (0=miter, 1=round, 2=bevel)
    SetLineJoin(u8),
    /// Set miter limit
    SetMiterLimit(f32),
    /// Set global alpha
    SetGlobalAlpha(f32),
    /// Set font
    SetFont { family: String, size: f32, bold: bool, italic: bool },
    /// Set text alignment (0=left, 1=center, 2=right, 3=start, 4=end)
    SetTextAlign(u8),
    /// Set text baseline (0=top, 1=hanging, 2=middle, 3=alphabetic, 4=ideographic, 5=bottom)
    SetTextBaseline(u8),
    
    // ========== Transform Commands ==========
    /// Set the transformation matrix
    SetTransform { a: f32, b: f32, c: f32, d: f32, e: f32, f: f32 },
    /// Reset to identity matrix
    ResetTransform,
    /// Translate
    Translate { x: f32, y: f32 },
    /// Rotate (radians)
    Rotate(f32),
    /// Scale
    Scale { x: f32, y: f32 },
    
    // ========== Path Commands ==========
    /// Begin a new path
    BeginPath,
    /// Close the current subpath
    ClosePath,
    /// Move to point
    MoveTo { x: f32, y: f32 },
    /// Line to point
    LineTo { x: f32, y: f32 },
    /// Quadratic curve to
    QuadraticCurveTo { cpx: f32, cpy: f32, x: f32, y: f32 },
    /// Bezier curve to
    BezierCurveTo { cp1x: f32, cp1y: f32, cp2x: f32, cp2y: f32, x: f32, y: f32 },
    /// Arc
    Arc { x: f32, y: f32, radius: f32, start_angle: f32, end_angle: f32, ccw: bool },
    /// Arc to
    ArcTo { x1: f32, y1: f32, x2: f32, y2: f32, radius: f32 },
    /// Rectangle path
    Rect { x: f32, y: f32, w: f32, h: f32 },
    /// Ellipse
    Ellipse { x: f32, y: f32, rx: f32, ry: f32, rotation: f32, start: f32, end: f32, ccw: bool },
    
    // ========== Draw Commands ==========
    /// Fill the current path
    Fill,
    /// Stroke the current path
    Stroke,
    /// Set clipping region to current path
    Clip,
    /// Fill a rectangle (optimized, no path needed)
    FillRect { x: f32, y: f32, w: f32, h: f32 },
    /// Stroke a rectangle
    StrokeRect { x: f32, y: f32, w: f32, h: f32 },
    /// Clear a rectangle
    ClearRect { x: f32, y: f32, w: f32, h: f32 },
    /// Fill text
    FillText { text: String, x: f32, y: f32, max_width: Option<f32> },
    /// Stroke text
    StrokeText { text: String, x: f32, y: f32, max_width: Option<f32> },
    /// Draw an image
    DrawImage {
        image_id: u32,
        sx: f32, sy: f32, sw: f32, sh: f32,
        dx: f32, dy: f32, dw: f32, dh: f32,
    },
    /// Batch draw multiple images (optimized)
    DrawImageBatch(Vec<DrawImageParams>),
}

/// Parameters for a single image draw in a batch
#[derive(Debug, Clone, Copy)]
pub struct DrawImageParams {
    pub image_id: u32,
    pub sx: f32, pub sy: f32, pub sw: f32, pub sh: f32,
    pub dx: f32, pub dy: f32, pub dw: f32, pub dh: f32,
}

/// WebGL command
///
/// These map closely to WebGL API calls but are batched for efficiency.
#[derive(Debug, Clone)]
pub enum GLCommand {
    // ========== State ==========
    Viewport { x: i32, y: i32, width: u32, height: u32 },
    Clear { mask: u32 },
    ClearColor { r: f32, g: f32, b: f32, a: f32 },
    Enable(u32),
    Disable(u32),
    BlendFunc { sfactor: u32, dfactor: u32 },
    DepthFunc(u32),
    CullFace(u32),
    
    // ========== Shaders & Programs ==========
    UseProgram(u32),
    
    // ========== Uniforms ==========
    Uniform1i { location: u32, value: i32 },
    Uniform1f { location: u32, value: f32 },
    Uniform2f { location: u32, x: f32, y: f32 },
    Uniform3f { location: u32, x: f32, y: f32, z: f32 },
    Uniform4f { location: u32, x: f32, y: f32, z: f32, w: f32 },
    UniformMatrix3fv { location: u32, transpose: bool, data: Vec<f32> },
    UniformMatrix4fv { location: u32, transpose: bool, data: Vec<f32> },
    
    // ========== Buffers ==========
    BindBuffer { target: u32, buffer: u32 },
    BufferData { target: u32, data: Vec<u8>, usage: u32 },
    BufferSubData { target: u32, offset: i32, data: Vec<u8> },
    
    // ========== Vertex Attributes ==========
    BindVertexArray(u32),
    EnableVertexAttribArray(u32),
    DisableVertexAttribArray(u32),
    VertexAttribPointer {
        index: u32,
        size: i32,
        type_: u32,
        normalized: bool,
        stride: i32,
        offset: i32,
    },
    
    // ========== Textures ==========
    ActiveTexture(u32),
    BindTexture { target: u32, texture: u32 },
    TexImage2D {
        target: u32,
        level: i32,
        internal_format: i32,
        width: i32,
        height: i32,
        format: u32,
        type_: u32,
        data: Option<Vec<u8>>,
    },
    TexParameteri { target: u32, pname: u32, param: i32 },
    
    // ========== Framebuffers ==========
    BindFramebuffer { target: u32, framebuffer: u32 },
    FramebufferTexture2D {
        target: u32,
        attachment: u32,
        textarget: u32,
        texture: u32,
        level: i32,
    },
    
    // ========== Drawing ==========
    DrawArrays { mode: u32, first: i32, count: i32 },
    DrawElements { mode: u32, count: i32, type_: u32, offset: i32 },
    
    // ========== Batched Drawing (optimization) ==========
    /// Draw multiple meshes with same shader but different transforms/uniforms
    DrawBatch(Vec<DrawCall>),
}

/// A single draw call in a batch
#[derive(Debug, Clone)]
pub struct DrawCall {
    pub mode: u32,
    pub first: i32,
    pub count: i32,
    /// Per-draw uniforms (location, value)
    pub uniforms: Vec<(u32, UniformValue)>,
}

/// Uniform value types
#[derive(Debug, Clone)]
pub enum UniformValue {
    Int(i32),
    Float(f32),
    Vec2(f32, f32),
    Vec3(f32, f32, f32),
    Vec4(f32, f32, f32, f32),
    Mat3(Vec<f32>),
    Mat4(Vec<f32>),
}
