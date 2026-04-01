extern crate khronos_egl as egl;

use std::collections::HashMap;

use glow::{
    NativeBuffer, NativeFramebuffer, NativeProgram, NativeRenderbuffer, NativeShader, NativeTexture,
};
use shared::error::{EngineError, ErrorCode};
use shared::protocol::render_cmd::{CanvasId, ProgramId, ShaderId, ShaderType};

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
    #[allow(dead_code)]
    pub is_onscreen: bool,
}

impl CanvasInfo {
    #[allow(dead_code)]
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Tracked WebGL state per canvas for deduplication of redundant GL calls.
///
/// When a setter call matches the already-tracked value, the actual GL call
/// is skipped.  State is only updated AFTER resource validation succeeds,
/// so errors never pollute the tracked state.
#[derive(Clone, Debug, Default)]
pub(crate) struct CanvasGLState {
    pub current_program: Option<ProgramId>,
    pub viewport: Option<(i32, i32, i32, i32)>,
    /// TEXTURE_2D binding per texture unit.  Key = GL enum (TEXTURE0..TEXTURE31).
    pub bound_texture_2d: HashMap<u32, Option<u32>>,
    /// ARRAY_BUFFER binding.
    pub bound_array_buffer: Option<Option<u32>>,
    /// ELEMENT_ARRAY_BUFFER binding.
    pub bound_element_array_buffer: Option<Option<u32>>,
    /// Active texture unit.
    pub active_texture_unit: Option<u32>,
}

#[derive(Debug)]
pub(crate) struct ProgramMeta {
    pub gl_handle: Option<NativeProgram>,
    pub owner_canvas: Option<CanvasId>, // None => resource context created
    pub deleted: bool,
    /// Shader IDs attached via `glAttachShader`.  Used by shader cache to
    /// reconstruct the cache key at link time.
    pub attached_shaders: Vec<ShaderId>,
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
    /// Shader source cached for shader binary cache key.
    pub source: Option<String>,
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
    #[allow(dead_code)]
    pub owner_canvas: Option<CanvasId>,
    pub deleted: bool,
}

#[derive(Debug)]
pub(crate) struct FramebufferMeta {
    pub gl_handle: Option<NativeFramebuffer>,
    #[allow(dead_code)]
    pub owner_canvas: Option<CanvasId>,
    pub deleted: bool,
}

#[derive(Debug)]
pub(crate) struct RenderbufferMeta {
    pub gl_handle: Option<NativeRenderbuffer>,
    #[allow(dead_code)]
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

pub(super) struct CanvasEntry {
    pub info: CanvasInfo,
    /// Actual EGL surface dimensions (physical pixels).
    pub physical_width: u32,
    pub physical_height: u32,
    pub kind: SurfaceKind,
    pub ctx: EglContextHandle,
    /// DrawingBuffer for the onscreen canvas. None for offscreen pbuffer canvases.
    pub drawing_buffer: Option<super::drawing_buffer::DrawingBuffer>,
    /// When true, WebGL renders directly to FBO 0 (window surface) and the
    /// DrawingBuffer blit is skipped.  Currently enabled when there is
    /// exactly one canvas (the onscreen one) with a DrawingBuffer.
    ///
    /// **Limitation:** `preserveDrawingBuffer` and `readPixels`/`toDataURL`
    /// on the default framebuffer are not yet gated — those features are
    /// uncommon in mini-game workloads.  Future phases should add runtime
    /// detection and disable bypass when needed.
    ///
    /// Set by `CanvasManager::evaluate_bypass()` after canvas lifecycle events.
    pub bypass_drawing_buffer: bool,
}
