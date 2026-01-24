use crossbeam_channel::Sender;
use femtovg::Color;
use tokio::sync::oneshot;

use crate::error::{EngineError, ErrorCode};
use crate::protocol::io_cmd::NormalizedImage;
use crate::surface::SurfaceRef;

pub type CanvasId = u32;
pub type ImageId = u32;
pub type ProgramId = u32;
pub type ShaderId = u32;
pub type BufferId = u32;
pub type Context2DId = u32;

/// Protocol-wide Render result type.
pub type RenderResult<T> = Result<T, EngineError>;

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

    Canvas(CanvasCmd),
    GL(GLCmd),

    Canvas2D {
        canvas_id: CanvasId,
        cmd: Canvas2DCmd,
    },
}

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

    /// Load an RGBA8 image (width/height + raw bytes).
    /// The render thread owns the GPU resource.
    LoadImage {
        image_id: ImageId,
        image: NormalizedImage,
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
        resp: RenderCmdResp<ProgramId>,
    },

    UseProgram {
        canvas_id: CanvasId,
        program_id: ProgramId,
    },

    LinkProgram {
        program_id: ProgramId,
        resp: RenderCmdResp<bool>,
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
        shader_type: ShaderType,
        resp: RenderCmdResp<ShaderId>,
    },

    ShaderSource {
        shader_id: ShaderId,
        source: String,
        resp: RenderCmdResp<()>,
    },

    CompileShader {
        shader_id: ShaderId,
        resp: RenderCmdResp<bool>,
    },

    AttachShader {
        program_id: ProgramId,
        shader_id: ShaderId,
        resp: RenderCmdResp<()>,
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
        resp: RenderCmdResp<BufferId>,
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
}

#[non_exhaustive]
#[derive(Debug)]
pub enum Canvas2DCmd {
    CreateContext2D {
        resp: RenderCmdResp<Context2DId>,
    },

    Arc {
        x: f32,
        y: f32,
        radius: f32,
        start_angle: f32,
        end_angle: f32,
        counterclockwise: bool,
    },

    SetFillStyle {
        color: Color,
    },
    SetStrokeStyle {
        color: Color,
    },
    SetLineWidth {
        width: f32,
    },

    FillRect {
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
    StrokeRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },

    BeginPath,

    MoveTo {
        x: f32,
        y: f32,
    },
    LineTo {
        x: f32,
        y: f32,
    },

    ClosePath,
    Fill,
    Stroke,
    Save,
    Restore,

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

    SetFont {
        font: String,
    },

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

    SetTransform {
        a: f32,
        b: f32,
        c: f32,
        d: f32,
        e: f32,
        f: f32,
    },
    ResetTransform,
}
