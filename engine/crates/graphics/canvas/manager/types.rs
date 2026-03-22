extern crate khronos_egl as egl;

use glow::{NativeBuffer, NativeFramebuffer, NativeProgram, NativeRenderbuffer, NativeShader, NativeTexture};
use shared::error::{EngineError, ErrorCode};
use shared::protocol::render_cmd::{CanvasId, ProgramId, ShaderType};

#[inline]
pub(crate) fn ee(code: ErrorCode, detail: impl Into<String>) -> EngineError {
    EngineError::from_detail(code, detail)
}

#[derive(Clone, Debug)]
pub(crate) struct CanvasInfo {
    #[allow(dead_code)]
    pub id: CanvasId,
    pub width: u32,
    pub height: u32,
    pub is_onscreen: bool,
}

impl CanvasInfo {
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CanvasGLState {
    pub current_program: Option<ProgramId>,
}

#[derive(Debug)]
pub(crate) struct ProgramMeta {
    pub gl_handle: Option<NativeProgram>,
    pub owner_canvas: Option<CanvasId>, // None => resource context created
    pub deleted: bool,
}

#[derive(Debug)]
pub(crate) struct ShaderMeta {
    pub gl_handle: Option<NativeShader>,
    pub owner_canvas: Option<CanvasId>,
    #[allow(dead_code)]
    pub shader_type: ShaderType, // Vertex / Fragment (protocol enum)
    pub gl_shader_type: u32, // glow::VERTEX_SHADER / glow::FRAGMENT_SHADER
    pub deleted: bool,
    pub source_len: usize, // cached for SHADER_SOURCE_LENGTH
}

#[derive(Debug)]
pub(crate) struct BufferMeta {
    pub gl_handle: Option<NativeBuffer>,
    pub owner_canvas: Option<CanvasId>,
    pub deleted: bool,
}

#[derive(Debug)]
pub(crate) struct TextureMeta {
    pub gl_handle: Option<NativeTexture>,
    pub owner_canvas: Option<CanvasId>,
    pub deleted: bool,
}

#[derive(Debug)]
pub(crate) struct FramebufferMeta {
    pub gl_handle: Option<NativeFramebuffer>,
    pub owner_canvas: Option<CanvasId>,
    pub deleted: bool,
}

#[derive(Debug)]
pub(crate) struct RenderbufferMeta {
    pub gl_handle: Option<NativeRenderbuffer>,
    pub owner_canvas: Option<CanvasId>,
    pub deleted: bool,
}

#[derive(Clone)]
pub(super) struct EglContextHandle {
    pub ctx: egl::Context,
    pub surf: egl::Surface,
}

#[derive(Clone, Copy)]
pub(super) enum SurfaceKind {
    Window(usize),
    Pbuffer,
}

#[derive(Clone)]
pub(super) struct CanvasEntry {
    pub info: CanvasInfo,
    pub logical_width: u32,
    pub logical_height: u32,
    pub kind: SurfaceKind,
    pub ctx: EglContextHandle,
}
