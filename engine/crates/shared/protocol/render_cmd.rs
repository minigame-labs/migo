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
/// WebGL 2 Vertex Array Object id.  Also used by WebGL 1 games that opt
/// into the `OES_vertex_array_object` extension — the underlying engine
/// resource is the same.
pub type VaoId = u32;
/// WebGL 2 Sampler Object id (decouples filtering / wrap from the texture).
pub type SamplerId = u32;
/// WebGL 2 Fence Sync object.  Opaque handle into the render thread's
/// `GLSyncRegistry`; the JS side never sees the underlying `GLsync`.
pub type SyncId = u32;

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

    // ========================================================================
    // WebGL 2.0 additions (GLES 3.0 backed).
    //
    // The protocol keeps WebGL 1 and 2 commands in the same enum so the
    // render-thread state tracker only has to look at one dispatch table.
    // WebGL 1 games that opt into extensions (`OES_vertex_array_object`,
    // `ANGLE_instanced_arrays`) end up emitting the same variants.
    // ========================================================================

    /// `createVertexArray()` — allocate a VAO.  The client-chosen id is
    /// passed down (matching how buffers/textures are allocated) so the
    /// op is fire-and-forget.
    CreateVertexArray {
        canvas_id: CanvasId,
        client_id: VaoId,
    },
    DeleteVertexArray {
        vao: VaoId,
    },
    /// `bindVertexArray(vao)` — passing `None` rebinds the default VAO.
    BindVertexArray {
        canvas_id: CanvasId,
        vao: Option<VaoId>,
    },

    // Instanced drawing — native in WebGL 2, extension-backed in WebGL 1.
    VertexAttribDivisor {
        canvas_id: CanvasId,
        index: u32,
        divisor: u32,
    },
    DrawArraysInstanced {
        canvas_id: CanvasId,
        mode: u32,
        first: i32,
        count: i32,
        instance_count: i32,
    },
    DrawElementsInstanced {
        canvas_id: CanvasId,
        mode: u32,
        count: i32,
        index_type: u32,
        offset: i32,
        instance_count: i32,
    },

    // Uniform Buffer Objects.
    /// `getUniformBlockIndex(program, name)` — replies with the block index
    /// (or `u32::MAX` for `GL_INVALID_INDEX`).
    GetUniformBlockIndex {
        program_id: ProgramId,
        name: String,
        resp: RenderCmdResp<u32>,
    },
    UniformBlockBinding {
        program_id: ProgramId,
        uniform_block_index: u32,
        uniform_block_binding: u32,
    },
    BindBufferBase {
        canvas_id: CanvasId,
        target: u32,
        index: u32,
        buffer: Option<BufferId>,
    },
    BindBufferRange {
        canvas_id: CanvasId,
        target: u32,
        index: u32,
        buffer: Option<BufferId>,
        offset: i32,
        size: i32,
    },

    // Immutable texture storage (faster than `texImage2D` chains).
    TexStorage2D {
        canvas_id: CanvasId,
        target: u32,
        levels: i32,
        internal_format: u32,
        width: i32,
        height: i32,
    },

    // Framebuffer ops.
    /// `blitFramebuffer(srcX0, srcY0, srcX1, srcY1, dstX0, dstY0, dstX1, dstY1, mask, filter)`.
    BlitFramebuffer {
        canvas_id: CanvasId,
        src_x0: i32,
        src_y0: i32,
        src_x1: i32,
        src_y1: i32,
        dst_x0: i32,
        dst_y0: i32,
        dst_x1: i32,
        dst_y1: i32,
        mask: u32,
        filter: u32,
    },
    /// `invalidateFramebuffer(target, attachments)` — tells the tiled GPU
    /// it can drop the contents of the listed attachments without writeback.
    InvalidateFramebuffer {
        canvas_id: CanvasId,
        target: u32,
        attachments: Vec<u32>,
    },
    RenderbufferStorageMultisample {
        canvas_id: CanvasId,
        target: u32,
        samples: i32,
        internal_format: u32,
        width: i32,
        height: i32,
    },

    // Sampler objects.
    CreateSampler {
        canvas_id: CanvasId,
        client_id: SamplerId,
    },
    DeleteSampler {
        sampler: SamplerId,
    },
    BindSampler {
        canvas_id: CanvasId,
        unit: u32,
        sampler: Option<SamplerId>,
    },
    SamplerParameteri {
        sampler: SamplerId,
        pname: u32,
        param: i32,
    },
    SamplerParameterf {
        sampler: SamplerId,
        pname: u32,
        param: f32,
    },

    // Sync objects — used for non-blocking readPixels / transfer-complete
    // probing.  The engine assigns a SyncId; the Rust side owns the raw
    // GLsync handle and never exposes it to JS.
    FenceSync {
        canvas_id: CanvasId,
        client_id: SyncId,
        condition: u32,
        flags: u32,
    },
    DeleteSync {
        sync: SyncId,
    },
    /// `clientWaitSync(sync, flags, timeout_ns)` — returns one of the
    /// `GL_ALREADY_SIGNALED`, `GL_CONDITION_SATISFIED`, `GL_TIMEOUT_EXPIRED`,
    /// or `GL_WAIT_FAILED` enums.
    ClientWaitSync {
        sync: SyncId,
        flags: u32,
        timeout_ns: u64,
        resp: RenderCmdResp<u32>,
    },

    // Draw-buffer selection (FBO multiple-render-targets).
    DrawBuffers {
        canvas_id: CanvasId,
        buffers: Vec<u32>,
    },
    ReadBuffer {
        canvas_id: CanvasId,
        src: u32,
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

/// Canvas 2D `direction` drawing state.  Controls bidirectional text
/// reordering and the resolution of `textAlign=start`/`end`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TextDirection {
    /// CSS `direction: inherit` — falls back to LTR at the context
    /// level since the engine has no parent element to inherit from.
    #[default]
    Inherit,
    /// Left-to-right: Latin, CJK, most scripts.
    Ltr,
    /// Right-to-left: Arabic, Hebrew.
    Rtl,
}

/// Result of measureText operation.
///
/// Serialised with camelCase field names so JS consumers receive the
/// exact property shape Canvas 2D specifies (`actualBoundingBoxLeft`,
/// `fontBoundingBoxAscent`, etc.), while Rust code continues to use
/// idiomatic snake_case.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextMetrics {
    /// Advance width of the run, in CSS pixels (Canvas 2D spec: the
    /// "width of a line of text that wraps".)
    pub width: f32,
    /// Distance from `x` anchor to the left edge of the tight glyph
    /// bounding box.
    pub actual_bounding_box_left: f32,
    /// Distance from `x` anchor to the right edge of the tight glyph
    /// bounding box.
    pub actual_bounding_box_right: f32,
    /// Distance from `y` anchor (baseline) to the top of the tight
    /// glyph bounding box.
    pub actual_bounding_box_ascent: f32,
    /// Distance from `y` anchor (baseline) to the bottom of the tight
    /// glyph bounding box.
    pub actual_bounding_box_descent: f32,
    /// Distance from `y` anchor to the top of the font's em box.
    pub font_bounding_box_ascent: f32,
    /// Distance from `y` anchor to the bottom of the font's em box.
    pub font_bounding_box_descent: f32,
    /// Distance from `y` anchor to the top of the em-height above the
    /// baseline (equivalent to `ascent` in CSS line-box).
    pub em_height_ascent: f32,
    /// Distance from `y` anchor to the bottom of the em-height below
    /// the baseline.
    pub em_height_descent: f32,
    /// Distance from `y` anchor to the hanging baseline (used for
    /// Devanagari / similar scripts).
    pub hanging_baseline: f32,
    /// Distance from `y` anchor to the alphabetic baseline — always
    /// zero because that's where the `y` anchor sits by definition,
    /// exposed for spec parity.
    pub alphabetic_baseline: f32,
    /// Distance from `y` anchor to the ideographic baseline (used for
    /// CJK metrics).
    pub ideographic_baseline: f32,
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
    /// `ctx.direction = "ltr" | "rtl" | "inherit"` — controls BiDi
    /// reordering and how `textAlign=start`/`end` resolve.
    SetTextDirection {
        direction: TextDirection,
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
#[derive(Debug, Clone, PartialEq)]
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

// ---------------------------------------------------------------------------
// Approximate deep-size accounting (for frame-collector budgeting)
// ---------------------------------------------------------------------------
//
// `size_of::<Cmd>()` alone misses every `Vec<u8>` / `String` / `Arc<Vec<u8>>`
// payload carried by the variant.  The frame collector's 4MB soft budget
// used to tick up by the enum-wrapper size per push, which for a single
// `bufferData(8MB mesh)` reported ~200 bytes instead of 8MB — so auto-flush
// never fired and the JS heap could grow unbounded.  Chromium's
// `CanvasResourceProvider::EstimatedSizeInBytes` walks recorded ops and
// their pinned resources for the same reason: only pessimistic byte
// estimates give a useful backpressure signal.
//
// The methods below return:
//     size_of::<Self>()  +  sum of heap-owned storage on the live variant
//
// Heap storage uses `capacity()` (not `len()`) because growth-over-shrink
// inside the collector doesn't release bytes, and the ring-vec the
// collector holds retains original capacities.

impl GLCmd {
    /// Return the canvas this command targets, if any.  Resource-context
    /// commands (shader/program create/delete/link, contextless metadata
    /// queries) return `None`; they don't dirty any per-canvas Skia
    /// cache and therefore don't need to mark any `Canvas2DContext`
    /// stale after the WebGL batch completes.
    ///
    /// Used by the render thread's `execute_gl_batch` to narrow down
    /// which `Canvas2DContext::skia_state_stale` flags to flip, in
    /// place of the previous over-conservative broadcast to every
    /// live 2D context.
    pub fn touches_canvas(&self) -> Option<CanvasId> {
        // Exhaustive over every variant that carries a `canvas_id`
        // field — enumerated so the render thread's per-context
        // stale marking picks up EVERY state-mutating command, not
        // just the hot ones.  Variants without `canvas_id`
        // (resource-context ops like `CreateShader`/`LinkProgram`
        // that don't touch per-canvas GL binding state) return
        // `None`; those can't dirty a Canvas2DContext's Skia
        // tracking on their own.
        //
        // Any new GLCmd variant with a `canvas_id` field MUST be
        // added here — otherwise `execute_gl_batch`'s scoped stale
        // marking silently fails to flip the right flag, and the
        // subsequent Canvas2D draw would trust stale Skia state.
        match self {
            GLCmd::Viewport { canvas_id, .. }
            | GLCmd::Clear { canvas_id, .. }
            | GLCmd::ClearColor { canvas_id, .. }
            | GLCmd::ClearDepth { canvas_id, .. }
            | GLCmd::ClearStencil { canvas_id, .. }
            | GLCmd::CreateProgram { canvas_id, .. }
            | GLCmd::CreateShader { canvas_id, .. }
            | GLCmd::UseProgram { canvas_id, .. }
            | GLCmd::DrawArrays { canvas_id, .. }
            | GLCmd::DrawElements { canvas_id, .. }
            | GLCmd::GetAttribLocation { canvas_id, .. }
            | GLCmd::GetActiveAttrib { canvas_id, .. }
            | GLCmd::GetActiveUniform { canvas_id, .. }
            | GLCmd::EnableVertexAttribArray { canvas_id, .. }
            | GLCmd::DisableVertexAttribArray { canvas_id, .. }
            | GLCmd::VertexAttribPointer { canvas_id, .. }
            | GLCmd::VertexAttribDivisor { canvas_id, .. }
            | GLCmd::CreateBuffer { canvas_id, .. }
            | GLCmd::BindBuffer { canvas_id, .. }
            | GLCmd::BufferData { canvas_id, .. }
            | GLCmd::BufferSubData { canvas_id, .. }
            | GLCmd::GetUniformLocation { canvas_id, .. }
            | GLCmd::Enable { canvas_id, .. }
            | GLCmd::Disable { canvas_id, .. }
            | GLCmd::IsEnabled { canvas_id, .. }
            | GLCmd::ActiveTexture { canvas_id, .. }
            | GLCmd::CreateTexture { canvas_id, .. }
            | GLCmd::BindTexture { canvas_id, .. }
            | GLCmd::TexParameteri { canvas_id, .. }
            | GLCmd::TexParameterf { canvas_id, .. }
            | GLCmd::GenerateMipmap { canvas_id, .. }
            | GLCmd::PixelStorei { canvas_id, .. }
            | GLCmd::BlendFunc { canvas_id, .. }
            | GLCmd::BlendFuncSeparate { canvas_id, .. }
            | GLCmd::BlendEquation { canvas_id, .. }
            | GLCmd::BlendEquationSeparate { canvas_id, .. }
            | GLCmd::BlendColor { canvas_id, .. }
            | GLCmd::DepthFunc { canvas_id, .. }
            | GLCmd::DepthMask { canvas_id, .. }
            | GLCmd::DepthRange { canvas_id, .. }
            | GLCmd::CullFace { canvas_id, .. }
            | GLCmd::FrontFace { canvas_id, .. }
            | GLCmd::LineWidth { canvas_id, .. }
            | GLCmd::PolygonOffset { canvas_id, .. }
            | GLCmd::StencilFunc { canvas_id, .. }
            | GLCmd::StencilFuncSeparate { canvas_id, .. }
            | GLCmd::StencilOp { canvas_id, .. }
            | GLCmd::StencilOpSeparate { canvas_id, .. }
            | GLCmd::StencilMask { canvas_id, .. }
            | GLCmd::StencilMaskSeparate { canvas_id, .. }
            | GLCmd::ColorMask { canvas_id, .. }
            | GLCmd::Scissor { canvas_id, .. }
            | GLCmd::Hint { canvas_id, .. }
            | GLCmd::CreateFramebuffer { canvas_id, .. }
            | GLCmd::BindFramebuffer { canvas_id, .. }
            | GLCmd::CheckFramebufferStatus { canvas_id, .. }
            | GLCmd::FramebufferRenderbuffer { canvas_id, .. }
            | GLCmd::CreateRenderbuffer { canvas_id, .. }
            | GLCmd::BindRenderbuffer { canvas_id, .. }
            | GLCmd::RenderbufferStorage { canvas_id, .. }
            | GLCmd::RenderbufferStorageMultisample { canvas_id, .. }
            | GLCmd::ReadPixels { canvas_id, .. }
            | GLCmd::GetParameter { canvas_id, .. }
            | GLCmd::BlitFramebuffer { canvas_id, .. }
            | GLCmd::InvalidateFramebuffer { canvas_id, .. }
            | GLCmd::CreateSampler { canvas_id, .. }
            | GLCmd::BindSampler { canvas_id, .. }
            | GLCmd::CreateVertexArray { canvas_id, .. }
            | GLCmd::BindVertexArray { canvas_id, .. }
            | GLCmd::DrawArraysInstanced { canvas_id, .. }
            | GLCmd::DrawElementsInstanced { canvas_id, .. }
            | GLCmd::BindBufferBase { canvas_id, .. }
            | GLCmd::BindBufferRange { canvas_id, .. }
            | GLCmd::DrawBuffers { canvas_id, .. }
            | GLCmd::ReadBuffer { canvas_id, .. }
            | GLCmd::FenceSync { canvas_id, .. } => Some(*canvas_id),

            // Everything else — resource-context commands (shader
            // create/source/compile/link, program create/attach/
            // link/delete, buffer/texture/sampler/vao/fbo/rbo
            // delete, uniform calls routed via `UseProgram`'s
            // canvas_id above, etc.) doesn't bind any per-canvas
            // GL state that a Canvas2DContext cares about.  The
            // `#[non_exhaustive]` fall-through also catches any
            // future variant that forgets to add its canvas_id
            // here — conservative behaviour is "don't mark
            // anything stale"; a real state leak will surface as
            // a render-time bug and force us to add the variant.
            _ => None,
        }
    }

    /// Best-effort upper bound on the bytes this command retains, including
    /// heap-owned payload.  Callers use it as a soft signal to decide when
    /// to flush a barrier; accuracy within ~1 KB is fine.
    #[allow(clippy::too_many_lines)]
    pub fn approx_deep_size_bytes(&self) -> usize {
        let base = std::mem::size_of::<GLCmd>();
        base + match self {
            // Shader source strings (string pool per program).
            GLCmd::ShaderSource { source, .. } => source.capacity(),

            // Name strings carried to the render thread for lookup.
            // `GetUniformLocation` / `GetAttribLocation` / `GetUniformBlockIndex`
            // carry a `name: String`; `GetActiveAttrib` / `GetActiveUniform`
            // only have an index and return the name in the response,
            // so they have no outbound string payload.
            GLCmd::GetUniformLocation { name, .. }
            | GLCmd::GetAttribLocation { name, .. }
            | GLCmd::GetUniformBlockIndex { name, .. } => name.capacity(),

            // Buffer uploads — the dominant budget item for 3D games.
            // `BufferData.data` is optional (spec allows passing
            // `null` to reserve without upload).
            GLCmd::BufferData { data, .. } => data.as_ref().map_or(0, |v| v.capacity()),
            GLCmd::BufferSubData { data, .. } => data.capacity(),

            // Texture uploads (RGBA or compressed block).  `TexImage2D`
            // is optional data (reservation vs upload); `TexSubImage2D`
            // is always `Arc<Vec<u8>>` with a concrete payload.
            GLCmd::TexImage2D { data, .. } => {
                data.as_ref().map_or(0, |arc| arc.capacity())
            }
            GLCmd::TexSubImage2D { data, .. } => data.capacity(),
            GLCmd::CompressedTexImage2D { data, .. } => data.capacity(),
            GLCmd::CompressedTexSubImage2D { data, .. } => data.capacity(),

            // Uniform array uploads — scalar per element, but a
            // `uniform4fv(bones[100])` is 400 floats = 1.6 KB.
            GLCmd::Uniform1iv { value, .. }
            | GLCmd::Uniform2iv { value, .. }
            | GLCmd::Uniform3iv { value, .. }
            | GLCmd::Uniform4iv { value, .. } => value.capacity() * std::mem::size_of::<i32>(),
            GLCmd::Uniform1fv { value, .. }
            | GLCmd::Uniform2fv { value, .. }
            | GLCmd::Uniform3fv { value, .. }
            | GLCmd::Uniform4fv { value, .. }
            | GLCmd::UniformMatrix2fv { value, .. }
            | GLCmd::UniformMatrix3fv { value, .. }
            | GLCmd::UniformMatrix4fv { value, .. } => {
                value.capacity() * std::mem::size_of::<f32>()
            }

            // WebGL 2 framebuffer metadata arrays.
            GLCmd::InvalidateFramebuffer { attachments, .. } => {
                attachments.capacity() * std::mem::size_of::<u32>()
            }
            GLCmd::DrawBuffers { buffers, .. } => {
                buffers.capacity() * std::mem::size_of::<u32>()
            }

            // All other variants are pure scalars / Copy payloads — the
            // enum stack size already accounts for them.
            _ => 0,
        }
    }
}

impl Canvas2DCmd {
    /// See [`GLCmd::approx_deep_size_bytes`].  Canvas 2D variants own
    /// text strings and image-batch vectors; everything else is
    /// inline-scalar.
    #[allow(clippy::too_many_lines)]
    pub fn approx_deep_size_bytes(&self) -> usize {
        let base = std::mem::size_of::<Canvas2DCmd>();
        base + match self {
            Canvas2DCmd::FillText { text, .. }
            | Canvas2DCmd::StrokeText { text, .. }
            | Canvas2DCmd::MeasureText { text, .. } => text.capacity(),
            Canvas2DCmd::SetFont { font, .. } => font.capacity(),
            Canvas2DCmd::SetLineDash { segments } => {
                segments.capacity() * std::mem::size_of::<f32>()
            }
            Canvas2DCmd::SetFillStyleGradient { stops, .. }
            | Canvas2DCmd::SetStrokeStyleGradient { stops, .. } => {
                stops.capacity() * std::mem::size_of::<GradientStop>()
            }
            Canvas2DCmd::DrawImageBatch { draws } => {
                draws.capacity() * std::mem::size_of::<DrawImageEntry>()
            }
            // Everything else is Copy / scalar-only.
            _ => 0,
        }
    }
}

#[cfg(test)]
mod approx_size_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn scalar_variant_reports_just_enum_size() {
        let cmd = Canvas2DCmd::Save;
        assert_eq!(
            cmd.approx_deep_size_bytes(),
            std::mem::size_of::<Canvas2DCmd>(),
        );
    }

    #[test]
    fn fill_text_includes_string_capacity() {
        let mut s = String::with_capacity(1024);
        s.push_str("hello");
        let cmd = Canvas2DCmd::FillText {
            text: s,
            x: 0.0,
            y: 0.0,
            max_width: 0.0,
        };
        let size = cmd.approx_deep_size_bytes();
        assert!(
            size >= std::mem::size_of::<Canvas2DCmd>() + 1024,
            "got {size}, expected >= enum_size + 1024"
        );
    }

    #[test]
    fn buffer_data_covers_heap_bytes() {
        let data = vec![0u8; 8 * 1024 * 1024]; // 8 MB mesh
        let size_hint = data.len() as i32;
        let cmd = GLCmd::BufferData {
            canvas_id: CanvasId::from(1u32),
            target: 0x8892,
            size: size_hint,
            data: Some(data),
            usage: 0x88E4,
        };
        let size = cmd.approx_deep_size_bytes();
        // Must include the full 8 MB payload, not just the enum shell.
        assert!(
            size >= 8 * 1024 * 1024,
            "BufferData(8MB) reported only {size} bytes — budget will under-count"
        );
    }

    #[test]
    fn buffer_data_with_none_reports_only_enum() {
        // Spec-legal `bufferData(target, size, usage)` reserves
        // without upload — data is None.
        let cmd = GLCmd::BufferData {
            canvas_id: CanvasId::from(1u32),
            target: 0x8892,
            size: 1024,
            data: None,
            usage: 0x88E4,
        };
        assert_eq!(cmd.approx_deep_size_bytes(), std::mem::size_of::<GLCmd>());
    }

    #[test]
    fn tex_image_2d_with_none_data_reports_only_enum() {
        let cmd = GLCmd::TexImage2D {
            canvas_id: CanvasId::from(1u32),
            target: 0x0DE1,
            level: 0,
            internalformat: 0x1908,
            width: 0,
            height: 0,
            border: 0,
            format: 0x1908,
            type_: 0x1401,
            data: None,
        };
        assert_eq!(cmd.approx_deep_size_bytes(), std::mem::size_of::<GLCmd>());
    }

    #[test]
    fn tex_sub_image_2d_counts_arc_payload() {
        let data = Arc::new(vec![0u8; 256 * 1024]);
        let cmd = GLCmd::TexSubImage2D {
            canvas_id: CanvasId::from(1u32),
            target: 0x0DE1,
            level: 0,
            xoffset: 0,
            yoffset: 0,
            width: 256,
            height: 256,
            format: 0x1908,
            type_: 0x1401,
            data,
        };
        assert!(cmd.approx_deep_size_bytes() >= 256 * 1024);
    }

    #[test]
    fn compressed_tex_image_2d_counts_vec_capacity() {
        let cmd = GLCmd::CompressedTexImage2D {
            canvas_id: CanvasId::from(1u32),
            target: 0x0DE1,
            level: 0,
            internalformat: 0x8D64,
            width: 128,
            height: 128,
            border: 0,
            data: vec![0u8; 64 * 1024],
        };
        assert!(cmd.approx_deep_size_bytes() >= 64 * 1024);
    }

    #[test]
    fn uniform_matrix_4fv_counts_f32_slice() {
        // 3 matrices of 16 floats each = 48 floats = 192 bytes.
        let value = vec![0.0f32; 48];
        let cmd = GLCmd::UniformMatrix4fv {
            canvas_id: CanvasId::from(1u32),
            location: Some(0),
            transpose: false,
            value,
        };
        let size = cmd.approx_deep_size_bytes();
        assert!(size >= std::mem::size_of::<GLCmd>() + 48 * 4);
    }

    #[test]
    fn draw_image_batch_counts_entry_slots() {
        let draws: Vec<DrawImageEntry> = (0..100)
            .map(|_| DrawImageEntry {
                image_id: 1,
                sx: 0.0,
                sy: 0.0,
                sw: 1.0,
                sh: 1.0,
                dx: 0.0,
                dy: 0.0,
                dw: 1.0,
                dh: 1.0,
            })
            .collect();
        let expected = 100 * std::mem::size_of::<DrawImageEntry>();
        let cmd = Canvas2DCmd::DrawImageBatch { draws };
        assert!(cmd.approx_deep_size_bytes() >= expected);
    }

    // ---- touches_canvas (P1-2 stale marking) --------------------

    #[test]
    fn touches_canvas_returns_canvas_for_state_mutating_ops() {
        let cid = CanvasId::from(42u32);
        let cmd = GLCmd::UseProgram {
            canvas_id: cid,
            program_id: 1u32,
        };
        assert_eq!(cmd.touches_canvas(), Some(cid));

        let cmd = GLCmd::BindBuffer {
            canvas_id: cid,
            target: 0x8892,
            buffer: None,
        };
        assert_eq!(cmd.touches_canvas(), Some(cid));

        let cmd = GLCmd::DrawArrays {
            canvas_id: cid,
            mode: 0x0004,
            first: 0,
            count: 3,
        };
        assert_eq!(cmd.touches_canvas(), Some(cid));
    }

    #[test]
    fn touches_canvas_returns_none_for_resource_context_ops() {
        // `LinkProgram` / `CompileShader` / `AttachShader` run on
        // the resource EGL context and don't carry a canvas_id —
        // they can't bind per-canvas state, so no Canvas2DContext
        // needs invalidation.  The `execute_gl_batch` scoped stale-
        // marking relies on these returning None to skip the
        // broadcast overhead.
        let cmd = GLCmd::LinkProgram { program_id: 1u32 };
        assert_eq!(cmd.touches_canvas(), None);

        let cmd = GLCmd::CompileShader { shader_id: 1u32 };
        assert_eq!(cmd.touches_canvas(), None);
    }

    #[test]
    fn batch_with_two_canvases_collects_both_for_scoped_stale() {
        // Simulates `execute_gl_batch`'s collection loop: a mixed
        // batch touching two canvases MUST report both so neither
        // context's Skia cache gets left trusting stale state.
        let a = CanvasId::from(1u32);
        let b = CanvasId::from(2u32);
        let commands = vec![
            GLCmd::UseProgram {
                canvas_id: a,
                program_id: 1u32,
            },
            GLCmd::BindBuffer {
                canvas_id: a,
                target: 0x8892,
                buffer: None,
            },
            GLCmd::UseProgram {
                canvas_id: b,
                program_id: 2u32,
            },
            GLCmd::LinkProgram { program_id: 1u32 }, // no canvas_id
        ];
        let mut touched: std::collections::HashSet<CanvasId> =
            std::collections::HashSet::new();
        for cmd in &commands {
            if let Some(c) = cmd.touches_canvas() {
                touched.insert(c);
            }
        }
        assert_eq!(touched.len(), 2, "expected two distinct canvases");
        assert!(touched.contains(&a));
        assert!(touched.contains(&b));
    }

    #[test]
    fn batch_of_pure_resource_ops_touches_no_canvas() {
        // When a GL batch consists solely of resource-context ops,
        // no Canvas2DContext should be marked stale.
        let commands = vec![
            GLCmd::LinkProgram { program_id: 1u32 },
            GLCmd::CompileShader { shader_id: 2u32 },
            GLCmd::DeleteProgram { program_id: 3u32 },
        ];
        let mut touched: std::collections::HashSet<CanvasId> =
            std::collections::HashSet::new();
        for cmd in &commands {
            if let Some(c) = cmd.touches_canvas() {
                touched.insert(c);
            }
        }
        assert!(touched.is_empty());
    }

    #[test]
    fn touches_canvas_distinguishes_per_canvas_state() {
        let a = CanvasId::from(1u32);
        let b = CanvasId::from(2u32);
        let ca = GLCmd::ActiveTexture {
            canvas_id: a,
            unit: 0,
        };
        let cb = GLCmd::ActiveTexture {
            canvas_id: b,
            unit: 0,
        };
        assert_eq!(ca.touches_canvas(), Some(a));
        assert_eq!(cb.touches_canvas(), Some(b));
        assert_ne!(ca.touches_canvas(), cb.touches_canvas());
    }
}
