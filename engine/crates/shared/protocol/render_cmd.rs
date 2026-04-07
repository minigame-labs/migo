use std::sync::Arc;

use crossbeam_channel::Sender;
use tokio::sync::oneshot;

pub use crate::protocol::color::Color;

use crate::error::{EngineError, ErrorCode};
use crate::protocol::FramePacket;
use crate::surface::SurfaceRef;

pub type CanvasId = u32;
pub type ImageId = u32;
pub type ProgramId = u32;
pub type ShaderId = u32;
pub type BufferId = u32;
pub type TextureId = u32;
pub type FramebufferId = u32;
pub type RenderbufferId = u32;
pub type Context2DId = u32;

/// Protocol-wide Render result type.
pub type RenderResult<T> = Result<T, EngineError>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirtyRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug)]
pub struct CanvasBatchPayload {
    pub canvas_id: CanvasId,
    pub commands: Vec<Canvas2DCmd>,
    pub present: bool,
    pub dirty_rect: Option<DirtyRect>,
}

#[derive(Debug)]
pub struct GlBatchPayload {
    pub commands: Vec<GLCmd>,
}

pub type SyncResp<T> = Sender<RenderResult<T>>;

#[must_use]
#[derive(Debug)]
pub enum RenderCmdResp<T> {
    Async(oneshot::Sender<RenderResult<T>>),
    Sync(SyncResp<T>),
}

impl<T> RenderCmdResp<T> {
    #[inline]
    pub fn send(self, v: RenderResult<T>) {
        match self {
            RenderCmdResp::Async(tx) => {
                let _ = tx.send(v);
            }
            RenderCmdResp::Sync(tx) => {
                let _ = tx.send(v);
            }
        }
    }

    #[inline]
    pub fn ok(self, v: T) {
        self.send(Ok(v));
    }

    #[inline]
    pub fn err(self, e: EngineError) {
        self.send(Err(e));
    }

    #[inline]
    pub fn err_code(self, code: ErrorCode) {
        self.send(Err(EngineError::new(code)));
    }

    #[inline]
    pub fn err_msg(self, msg: impl Into<String>) {
        self.send(Err(
            EngineError::new(ErrorCode::RenderBackendError).with_detail(msg.into())
        ));
    }
}

/// Render thread commands.
#[non_exhaustive]
#[derive(Debug)]
pub enum RenderCommand {
    FrameRate(u32),
    Shutdown,
    FramePacket(FramePacket),

    Canvas(CanvasCmd),
    GL(GLCmd),
    /// Batched WebGL commands executed in-order by render thread.
    GLBatch(GlBatchPayload),

    /// Single Canvas2D command (V1 - immediate mode)
    Canvas2D {
        canvas_id: CanvasId,
        cmd: Canvas2DCmd,
    },

    /// Batched Canvas2D commands (V2 - command batching)
    ///
    /// Contains all Canvas2D commands for a single frame, sent as one message.
    /// This significantly reduces IPC overhead compared to sending each command individually.
    Canvas2DBatch(CanvasBatchPayload),

    /// Invalidate signal for on-demand rendering mode
    ///
    /// When in on-demand mode, the render thread only renders when:
    /// 1. Content has changed (commands received)
    /// 2. An explicit Invalidate signal is received
    Invalidate,

    /// Load a custom font from raw bytes into the global font store and all existing canvases.
    /// Returns the font family key (used in CSS font strings).
    LoadFont {
        key: String,
        bytes: std::sync::Arc<Vec<u8>>,
        resp: RenderCmdResp<String>,
    },

    /// Measure the line height for a given font configuration.
    /// Returns the line height in pixels (ascender - descender).
    GetTextLineHeight {
        font_family: String,
        font_size: f32,
        bold: bool,
        italic: bool,
        resp: RenderCmdResp<f32>,
    },

    /// Pause rendering: stop the frame ticker/VSync and RAF signal.
    ///
    /// Used when the app goes to background. The render thread stays alive
    /// and continues processing commands (e.g., `RecreateOnscreen`), but
    /// stops producing frames and sending RAF timestamps to the JS op.
    Pause,

    /// Resume rendering: restart the RAF ticker and frame presentation.
    ///
    /// Used when the app returns to foreground.
    Resume,

    /// Mark the current onscreen surface as destroyed.
    ///
    /// The render thread keeps running and can still accept a later
    /// `RecreateOnscreen`, but must stop presenting until then.
    SurfaceDestroyed,
}

// Guard against future regressions — if a new variant re-inflates the enum,
// this assertion will fail at compile time.
const _: () = assert!(
    core::mem::size_of::<RenderCommand>() <= 128,
    "RenderCommand grew past 128 bytes; check for unboxed large variants"
);

#[non_exhaustive]
#[derive(Debug)]
pub enum CanvasCmd {
    CreateOffscreen {
        width: u32,
        height: u32,
        resp: RenderCmdResp<CanvasId>,
    },

    DestroyCanvas {
        id: CanvasId,
        resp: RenderCmdResp<()>,
    },

    RecreateOnscreen {
        surface: SurfaceRef,
        resp: RenderCmdResp<()>,
    },

    ResizeCanvas {
        id: CanvasId,
        w: Option<u32>,
        h: Option<u32>,
    },

    MakeCurrent {
        id: CanvasId,
        resp: RenderCmdResp<()>,
    },

    SwapBuffers {
        id: CanvasId,
        wait_for_vsync: bool,
        resp: RenderCmdResp<()>,
    },

    GetInfo {
        id: CanvasId,
        resp: RenderCmdResp<(u32, u32)>,
    },

    // Image resources (owned by render thread)
    CreateImage {
        resp: RenderCmdResp<ImageId>,
    },

    /// Load an image (RGBA8 or compressed) for GPU upload.
    /// The render thread owns the GPU resource.
    LoadImage {
        image_id: ImageId,
        image: super::io_cmd::DecodedImage,
        priority: super::io_cmd::ImagePriority,
        resp: RenderCmdResp<(u32, u32)>, // (width, height)
    },

    DestroyImage {
        image_id: ImageId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderType {
    Vertex,
    Fragment,
}

#[non_exhaustive]
#[derive(Debug)]
pub enum GLCmd {
    Viewport {
        canvas_id: CanvasId,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },

    Clear {
        canvas_id: CanvasId,
        bit_field: u32,
    },

    ClearColor {
        canvas_id: CanvasId,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    },

    // Program
    CreateProgram {
        canvas_id: CanvasId,
        client_id: ProgramId,
    },

    UseProgram {
        canvas_id: CanvasId,
        program_id: ProgramId,
    },

    LinkProgram {
        program_id: ProgramId,
    },

    GetProgramParameter {
        program_id: ProgramId,
        pname: u32,
        resp: RenderCmdResp<i32>,
    },

    GetProgramInfoLog {
        program_id: ProgramId,
        resp: RenderCmdResp<Option<String>>,
    },

    DeleteProgram {
        program_id: ProgramId,
    },

    // Shader
    CreateShader {
        canvas_id: CanvasId,
        client_id: ShaderId,
        shader_type: ShaderType,
    },

    ShaderSource {
        shader_id: ShaderId,
        source: String,
        /// `None` = fire-and-forget (batched path).
        resp: Option<RenderCmdResp<()>>,
    },

    CompileShader {
        shader_id: ShaderId,
    },

    AttachShader {
        program_id: ProgramId,
        shader_id: ShaderId,
        /// `None` = fire-and-forget (batched path).
        resp: Option<RenderCmdResp<()>>,
    },

    GetShaderParameter {
        shader_id: ShaderId,
        pname: u32,
        resp: RenderCmdResp<i32>,
    },

    GetShaderInfoLog {
        shader_id: ShaderId,
        resp: RenderCmdResp<Option<String>>,
    },

    DeleteShader {
        shader_id: ShaderId,
    },

    // Draw
    DrawArrays {
        canvas_id: CanvasId,
        mode: u32,
        first: i32,
        count: i32,
    },

    DrawElements {
        canvas_id: CanvasId,
        mode: u32,
        count: i32,
        index_type: u32,
        offset: i32,
    },

    // Attributes / uniforms
    GetAttribLocation {
        canvas_id: CanvasId,
        program_id: ProgramId,
        name: String,
        resp: RenderCmdResp<Option<u32>>,
    },

    GetActiveAttrib {
        canvas_id: CanvasId,
        program_id: ProgramId,
        index: u32,
        resp: RenderCmdResp<Option<(String, i32, u32)>>,
    },

    GetActiveUniform {
        canvas_id: CanvasId,
        program_id: ProgramId,
        index: u32,
        resp: RenderCmdResp<Option<(String, i32, u32)>>,
    },

    EnableVertexAttribArray {
        canvas_id: CanvasId,
        index: u32,
    },

    VertexAttribPointer {
        canvas_id: CanvasId,
        index: u32,
        size: i32,
        type_: u32,
        normalized: bool,
        stride: i32,
        offset: i32,
    },

    // Buffers
    CreateBuffer {
        canvas_id: CanvasId,
        client_id: BufferId,
    },

    BindBuffer {
        canvas_id: CanvasId,
        target: u32,
        buffer: Option<BufferId>,
    },

    BufferData {
        canvas_id: CanvasId,
        target: u32,
        size: i32,
        data: Option<Vec<u8>>,
        usage: u32,
    },

    GetUniformLocation {
        canvas_id: CanvasId,
        program_id: ProgramId,
        name: String,
        resp: RenderCmdResp<Option<u32>>,
    },

    Uniform3f {
        canvas_id: CanvasId,
        location: Option<u32>,
        x: f32,
        y: f32,
        z: f32,
    },

    UniformMatrix3fv {
        canvas_id: CanvasId,
        location: Option<u32>,
        transpose: bool,
        value: Vec<f32>,
    },

    // ========== Phase 1A: GL State ==========
    Enable {
        canvas_id: CanvasId,
        cap: u32,
    },
    Disable {
        canvas_id: CanvasId,
        cap: u32,
    },
    IsEnabled {
        canvas_id: CanvasId,
        cap: u32,
        resp: RenderCmdResp<bool>,
    },
    GetParameter {
        canvas_id: CanvasId,
        pname: u32,
        resp: RenderCmdResp<String>,
    },

    // ========== Phase 1B: Textures ==========
    CreateTexture {
        canvas_id: CanvasId,
        client_id: TextureId,
    },
    DeleteTexture {
        texture_id: TextureId,
    },
    BindTexture {
        canvas_id: CanvasId,
        target: u32,
        texture: Option<TextureId>,
    },
    ActiveTexture {
        canvas_id: CanvasId,
        unit: u32,
    },
    TexImage2D {
        canvas_id: CanvasId,
        target: u32,
        level: i32,
        internalformat: i32,
        width: i32,
        height: i32,
        border: i32,
        format: u32,
        type_: u32,
        data: Option<Arc<Vec<u8>>>,
    },
    TexSubImage2D {
        canvas_id: CanvasId,
        target: u32,
        level: i32,
        xoffset: i32,
        yoffset: i32,
        width: i32,
        height: i32,
        format: u32,
        type_: u32,
        data: Arc<Vec<u8>>,
    },
    TexParameteri {
        canvas_id: CanvasId,
        target: u32,
        pname: u32,
        param: i32,
    },
    TexParameterf {
        canvas_id: CanvasId,
        target: u32,
        pname: u32,
        param: f32,
    },
    GenerateMipmap {
        canvas_id: CanvasId,
        target: u32,
    },
    PixelStorei {
        canvas_id: CanvasId,
        pname: u32,
        param: i32,
    },
    CompressedTexImage2D {
        canvas_id: CanvasId,
        target: u32,
        level: i32,
        internalformat: u32,
        width: i32,
        height: i32,
        border: i32,
        data: Vec<u8>,
    },
    CompressedTexSubImage2D {
        canvas_id: CanvasId,
        target: u32,
        level: i32,
        xoffset: i32,
        yoffset: i32,
        width: i32,
        height: i32,
        format: u32,
        data: Vec<u8>,
    },

    // ========== Phase 1C: Buffer & Vertex Extensions ==========
    BufferSubData {
        canvas_id: CanvasId,
        target: u32,
        offset: i32,
        data: Vec<u8>,
    },
    DisableVertexAttribArray {
        canvas_id: CanvasId,
        index: u32,
    },
    ClearDepth {
        canvas_id: CanvasId,
        depth: f32,
    },
    ClearStencil {
        canvas_id: CanvasId,
        s: i32,
    },

    // ========== Phase 2A: Blend/Depth/Stencil/Cull State ==========
    BlendFunc {
        canvas_id: CanvasId,
        sfactor: u32,
        dfactor: u32,
    },
    BlendFuncSeparate {
        canvas_id: CanvasId,
        src_rgb: u32,
        dst_rgb: u32,
        src_alpha: u32,
        dst_alpha: u32,
    },
    BlendEquation {
        canvas_id: CanvasId,
        mode: u32,
    },
    BlendEquationSeparate {
        canvas_id: CanvasId,
        mode_rgb: u32,
        mode_alpha: u32,
    },
    BlendColor {
        canvas_id: CanvasId,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    },
    DepthFunc {
        canvas_id: CanvasId,
        func: u32,
    },
    DepthMask {
        canvas_id: CanvasId,
        flag: bool,
    },
    DepthRange {
        canvas_id: CanvasId,
        near: f32,
        far: f32,
    },
    StencilFunc {
        canvas_id: CanvasId,
        func: u32,
        ref_: i32,
        mask: u32,
    },
    StencilFuncSeparate {
        canvas_id: CanvasId,
        face: u32,
        func: u32,
        ref_: i32,
        mask: u32,
    },
    StencilOp {
        canvas_id: CanvasId,
        fail: u32,
        zfail: u32,
        zpass: u32,
    },
    StencilOpSeparate {
        canvas_id: CanvasId,
        face: u32,
        fail: u32,
        zfail: u32,
        zpass: u32,
    },
    StencilMask {
        canvas_id: CanvasId,
        mask: u32,
    },
    StencilMaskSeparate {
        canvas_id: CanvasId,
        face: u32,
        mask: u32,
    },
    CullFace {
        canvas_id: CanvasId,
        mode: u32,
    },
    FrontFace {
        canvas_id: CanvasId,
        mode: u32,
    },
    ColorMask {
        canvas_id: CanvasId,
        r: bool,
        g: bool,
        b: bool,
        a: bool,
    },
    Scissor {
        canvas_id: CanvasId,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
    LineWidth {
        canvas_id: CanvasId,
        width: f32,
    },
    PolygonOffset {
        canvas_id: CanvasId,
        factor: f32,
        units: f32,
    },

    // ========== Phase 2B: Uniform Variants ==========
    Uniform1i {
        canvas_id: CanvasId,
        location: Option<u32>,
        x: i32,
    },
    Uniform1f {
        canvas_id: CanvasId,
        location: Option<u32>,
        x: f32,
    },
    Uniform2f {
        canvas_id: CanvasId,
        location: Option<u32>,
        x: f32,
        y: f32,
    },
    Uniform4f {
        canvas_id: CanvasId,
        location: Option<u32>,
        x: f32,
        y: f32,
        z: f32,
        w: f32,
    },
    Uniform1iv {
        canvas_id: CanvasId,
        location: Option<u32>,
        value: Vec<i32>,
    },
    Uniform1fv {
        canvas_id: CanvasId,
        location: Option<u32>,
        value: Vec<f32>,
    },
    Uniform2iv {
        canvas_id: CanvasId,
        location: Option<u32>,
        value: Vec<i32>,
    },
    Uniform2fv {
        canvas_id: CanvasId,
        location: Option<u32>,
        value: Vec<f32>,
    },
    Uniform3iv {
        canvas_id: CanvasId,
        location: Option<u32>,
        value: Vec<i32>,
    },
    Uniform3fv {
        canvas_id: CanvasId,
        location: Option<u32>,
        value: Vec<f32>,
    },
    Uniform4iv {
        canvas_id: CanvasId,
        location: Option<u32>,
        value: Vec<i32>,
    },
    Uniform4fv {
        canvas_id: CanvasId,
        location: Option<u32>,
        value: Vec<f32>,
    },
    UniformMatrix2fv {
        canvas_id: CanvasId,
        location: Option<u32>,
        transpose: bool,
        value: Vec<f32>,
    },
    UniformMatrix4fv {
        canvas_id: CanvasId,
        location: Option<u32>,
        transpose: bool,
        value: Vec<f32>,
    },

    // ========== Phase 3A: Framebuffer/Renderbuffer ==========
    CreateFramebuffer {
        canvas_id: CanvasId,
        client_id: FramebufferId,
    },
    DeleteFramebuffer {
        framebuffer_id: FramebufferId,
    },
    BindFramebuffer {
        canvas_id: CanvasId,
        target: u32,
        framebuffer: Option<FramebufferId>,
    },
    FramebufferTexture2D {
        canvas_id: CanvasId,
        target: u32,
        attachment: u32,
        textarget: u32,
        texture: Option<TextureId>,
        level: i32,
    },
    FramebufferRenderbuffer {
        canvas_id: CanvasId,
        target: u32,
        attachment: u32,
        renderbuffertarget: u32,
        renderbuffer: Option<RenderbufferId>,
    },
    CheckFramebufferStatus {
        canvas_id: CanvasId,
        target: u32,
        resp: RenderCmdResp<u32>,
    },
    CreateRenderbuffer {
        canvas_id: CanvasId,
        client_id: RenderbufferId,
    },
    DeleteRenderbuffer {
        renderbuffer_id: RenderbufferId,
    },
    BindRenderbuffer {
        canvas_id: CanvasId,
        target: u32,
        renderbuffer: Option<RenderbufferId>,
    },
    RenderbufferStorage {
        canvas_id: CanvasId,
        target: u32,
        internalformat: u32,
        width: i32,
        height: i32,
    },

    // ========== Phase 3B: Misc ==========
    ReadPixels {
        canvas_id: CanvasId,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        format: u32,
        type_: u32,
        resp: RenderCmdResp<Vec<u8>>,
    },
    Hint {
        canvas_id: CanvasId,
        target: u32,
        mode: u32,
    },
}

/// Text horizontal alignment for fillText/strokeText.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Start,
    End,
    Left,
    Right,
    Center,
}

/// Text vertical baseline for fillText/strokeText.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextBaseline {
    Top,
    Hanging,
    Middle,
    #[default]
    Alphabetic,
    Ideographic,
    Bottom,
}

/// Result of measureText operation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TextMetrics {
    pub width: f32,
    pub actual_bounding_box_left: f32,
    pub actual_bounding_box_right: f32,
    pub actual_bounding_box_ascent: f32,
    pub actual_bounding_box_descent: f32,
    pub font_bounding_box_ascent: f32,
    pub font_bounding_box_descent: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientType {
    Linear,
    Radial,
    Conic,
}

#[non_exhaustive]
#[derive(Debug)]
pub enum Canvas2DCmd {
    CreateContext2D {
        resp: RenderCmdResp<Context2DId>,
    },

    // ========== Path methods ==========
    BeginPath,
    ClosePath,
    MoveTo {
        x: f32,
        y: f32,
    },
    LineTo {
        x: f32,
        y: f32,
    },
    QuadraticCurveTo {
        cpx: f32,
        cpy: f32,
        x: f32,
        y: f32,
    },
    BezierCurveTo {
        cp1x: f32,
        cp1y: f32,
        cp2x: f32,
        cp2y: f32,
        x: f32,
        y: f32,
    },
    Arc {
        x: f32,
        y: f32,
        radius: f32,
        start_angle: f32,
        end_angle: f32,
        counterclockwise: bool,
    },
    ArcTo {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        radius: f32,
    },
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    Ellipse {
        x: f32,
        y: f32,
        radius_x: f32,
        radius_y: f32,
        rotation: f32,
        start_angle: f32,
        end_angle: f32,
        counterclockwise: bool,
    },

    // ========== Drawing methods ==========
    Fill,
    Stroke,
    Clip,

    // ========== Rectangle methods ==========
    FillRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    StrokeRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    ClearRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },

    // ========== Text methods ==========
    FillText {
        text: String,
        x: f32,
        y: f32,
        max_width: f32,
    },
    StrokeText {
        text: String,
        x: f32,
        y: f32,
        max_width: f32,
    },
    MeasureText {
        text: String,
        resp: RenderCmdResp<TextMetrics>,
    },

    // ========== Style setters ==========
    SetFillStyle {
        color: Color,
    },
    SetStrokeStyle {
        color: Color,
    },
    SetLineWidth {
        width: f32,
    },
    SetLineCap {
        cap: u8,
    },
    SetLineJoin {
        join: u8,
    },
    SetMiterLimit {
        limit: f32,
    },
    SetGlobalAlpha {
        alpha: f32,
    },
    SetCompositeOperation {
        /// Encoded as u8: 0=source-over, 1=source-in, 2=source-out,
        /// 3=source-atop, 4=destination-over, 5=destination-in,
        /// 6=destination-out, 7=destination-atop, 8=lighter, 9=copy, 10=xor
        op: u8,
    },
    SetLineDash {
        /// Alternating dash/gap lengths. Empty = solid line.
        segments: Vec<f32>,
    },
    SetLineDashOffset {
        offset: f32,
    },
    SetShadowBlur {
        blur: f32,
    },
    SetShadowColor {
        color: Color,
    },
    SetShadowOffsetX {
        offset: f32,
    },
    SetShadowOffsetY {
        offset: f32,
    },
    SetFillStyleGradient {
        gradient_type: GradientType,
        x0: f32,
        y0: f32,
        r0: f32,
        x1: f32,
        y1: f32,
        r1: f32,
        stops: Vec<GradientStop>,
    },
    SetStrokeStyleGradient {
        gradient_type: GradientType,
        x0: f32,
        y0: f32,
        r0: f32,
        x1: f32,
        y1: f32,
        r1: f32,
        stops: Vec<GradientStop>,
    },
    SetFillStylePattern {
        image_id: ImageId,
        repeat_x: bool,
        repeat_y: bool,
    },
    SetStrokeStylePattern {
        image_id: ImageId,
        repeat_x: bool,
        repeat_y: bool,
    },
    SetFont {
        font: String,
    },
    SetTextAlign {
        align: TextAlign,
    },
    SetTextBaseline {
        baseline: TextBaseline,
    },

    // ========== State methods ==========
    Save,
    Restore,

    // ========== Transform methods ==========
    SetTransform {
        a: f32,
        b: f32,
        c: f32,
        d: f32,
        e: f32,
        f: f32,
    },
    ResetTransform,
    Translate {
        x: f32,
        y: f32,
    },
    Rotate {
        angle: f32,
    },
    Scale {
        x: f32,
        y: f32,
    },

    // ========== Image methods ==========
    DrawImage {
        image_id: ImageId,
        sx: f32,
        sy: f32,
        sw: f32,
        sh: f32,
        dx: f32,
        dy: f32,
        dw: f32,
        dh: f32,
    },
    GetImageData {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        resp: RenderCmdResp<Vec<u8>>,
    },

    /// Batch draw multiple images for better performance
    /// Each entry is (image_id, sx, sy, sw, sh, dx, dy, dw, dh)
    DrawImageBatch {
        draws: Vec<DrawImageEntry>,
    },
}

/// A single color stop for a linear/radial gradient.
#[derive(Debug, Clone)]
pub struct GradientStop {
    pub offset: f32,
    pub color: Color,
}

/// Single draw image entry for batch drawing
#[derive(Debug, Clone, Copy)]
pub struct DrawImageEntry {
    pub image_id: ImageId,
    pub sx: f32,
    pub sy: f32,
    pub sw: f32,
    pub sh: f32,
    pub dx: f32,
    pub dy: f32,
    pub dw: f32,
    pub dh: f32,
}
