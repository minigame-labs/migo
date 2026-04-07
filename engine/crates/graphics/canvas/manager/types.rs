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
/// Scissor test state for damage tracking.
///
/// GL allows `glScissor()` to be called at any time (even when the test is
/// disabled); the rect is retained and takes effect when the test is enabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScissorState {
    /// `GL_SCISSOR_TEST` is disabled — scissor rect has no effect.
    Disabled,
    /// `GL_SCISSOR_TEST` is enabled with a known rect (set by explicit `glScissor`).
    Enabled { x: i32, y: i32, width: i32, height: i32 },
    /// `GL_SCISSOR_TEST` is enabled but no explicit `glScissor()` has been
    /// called yet.  The real GL initial scissor box is the full drawable —
    /// we don't know its size here, so damage must fall back to viewport
    /// (conservative, never under-reports).
    EnabledUnknownRect,
}

/// When a setter call matches the already-tracked value, the actual GL call
/// is skipped.  State is only updated AFTER resource validation succeeds,
/// so errors never pollute the tracked state.
#[derive(Clone, Debug)]
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
    /// True when the DRAW_FRAMEBUFFER binding is the default framebuffer
    /// (= drawing buffer / onscreen surface).  False when a user FBO is bound.
    /// Used by damage tracking: only draws to the default FB affect onscreen damage.
    /// Defaults to true (initial GL state is the default framebuffer).
    pub draws_to_default_fbo: bool,
    /// Scissor test state for damage tracking.
    pub scissor: ScissorState,
    /// Last rect set via explicit `glScissor()`, retained across enable/disable
    /// cycles.  `None` means no explicit `glScissor` has been called yet.
    pub last_scissor_rect: Option<(i32, i32, i32, i32)>,
    /// Current glColorMask state (r, g, b, a).
    /// Used by damage tracking: if all four are false, glClear with
    /// COLOR_BUFFER_BIT doesn't actually modify visible color.
    /// Defaults to (true, true, true, true) per GL initial state.
    pub color_mask: (bool, bool, bool, bool),
}

impl Default for CanvasGLState {
    fn default() -> Self {
        Self {
            current_program: None,
            viewport: None,
            bound_texture_2d: HashMap::new(),
            bound_array_buffer: None,
            bound_element_array_buffer: None,
            active_texture_unit: None,
            // Initial GL state: default framebuffer is bound.
            draws_to_default_fbo: true,
            scissor: ScissorState::Disabled,
            last_scissor_rect: None,
            color_mask: (true, true, true, true),
        }
    }
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
    /// When bypass is active, the window surface content becomes undefined
    /// after `eglSwapBuffers` (per EGL spec). To handle games that read from
    /// the default framebuffer, `CanvasManager::signal_default_fbo_readback()`
    /// permanently disables bypass when such a readback is detected. This
    /// re-routes rendering through the DrawingBuffer which preserves content.
    ///
    /// Detection points: `ReadPixels` on the onscreen default FBO (GL handler)
    /// and `GetImageData` on canvas_id=1 (Canvas2D handler).
    ///
    /// Set by `CanvasManager::evaluate_bypass()` after canvas lifecycle events.
    pub bypass_drawing_buffer: bool,
}
