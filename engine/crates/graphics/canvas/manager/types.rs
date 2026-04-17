extern crate khronos_egl as egl;

use std::collections::{HashMap, HashSet};

use glow::{
    NativeBuffer, NativeFence, NativeFramebuffer, NativeProgram, NativeRenderbuffer,
    NativeSampler, NativeShader, NativeTexture, NativeVertexArray,
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

/// `(src_factor, dst_factor)` for glBlendFunc-style pairs.  Stored per
/// channel (colour vs alpha) to support `glBlendFuncSeparate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BlendFactors {
    pub src_rgb: u32,
    pub dst_rgb: u32,
    pub src_alpha: u32,
    pub dst_alpha: u32,
}

impl Default for BlendFactors {
    fn default() -> Self {
        // GL initial values: SRC = 1, DST = 0 for all channels.
        Self {
            src_rgb: 1,
            dst_rgb: 0,
            src_alpha: 1,
            dst_alpha: 0,
        }
    }
}

/// `(mode_rgb, mode_alpha)` for glBlendEquation / glBlendEquationSeparate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BlendEquation {
    pub mode_rgb: u32,
    pub mode_alpha: u32,
}

impl Default for BlendEquation {
    fn default() -> Self {
        // GL_FUNC_ADD = 0x8006 — GL's initial mode.
        Self {
            mode_rgb: 0x8006,
            mode_alpha: 0x8006,
        }
    }
}

/// Tracks the last value written to a given uniform `(program_id, location)`.
///
/// We compare raw bytes: two `glUniform4f` calls with the same three f32
/// values produce identical byte streams, so bytewise equality is both
/// necessary and sufficient to dedup a redundant upload.
///
/// Cache is bounded per-program to avoid unbounded growth when a game
/// spams glGetUniformLocation for hundreds of unique names; once a
/// program has more than [`MAX_UNIFORM_CACHE`] entries we drop the
/// oldest on insert.  Redundant uploads for evicted locations simply
/// fall back to the non-dedup path, never wrong.
pub(crate) const MAX_UNIFORM_CACHE: usize = 256;

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

    // ---- Extended dedup state (Phase 8) --------------------------------
    //
    // Everything below defaults to the initial GL ES 3.0 state per spec.
    // The state tracker uses these as cheap "already in this configuration"
    // checks before issuing the matching GL call.
    /// `glEnable(cap)` set.  Presence ⇒ enabled.  Absence ⇒ we don't know yet.
    pub enabled_caps: std::collections::HashSet<u32>,
    /// `glDisable(cap)` set — separated from `enabled_caps` so we can
    /// distinguish "known-disabled" from "never touched" (latter returns
    /// None from the lookup, forcing a real GL query to populate).
    pub disabled_caps: std::collections::HashSet<u32>,
    pub blend_factors: Option<BlendFactors>,
    pub blend_equation: Option<BlendEquation>,
    pub blend_color: Option<(f32, f32, f32, f32)>,
    pub depth_func: Option<u32>,
    pub depth_mask: Option<bool>,
    pub depth_range: Option<(f32, f32)>,
    pub cull_face: Option<u32>,
    pub front_face: Option<u32>,
    pub line_width: Option<f32>,
    pub polygon_offset: Option<(f32, f32)>,
    pub unpack_alignment: Option<i32>,
    pub pack_alignment: Option<i32>,
    /// Currently bound VAO (`None` = default vao 0).
    pub bound_vao: Option<u32>,
    /// Per-program uniform value cache.  Keys are `glGetUniformLocation`
    /// values (u32, stored as the location-index returned to JS).
    pub uniform_cache: HashMap<(ProgramId, u32), Box<[u8]>>,

    // ---- P11-state-tracker expansion ----------------------------------
    /// Currently bound FBO id for each GL binding target.  Keys are
    /// `glow::FRAMEBUFFER`, `glow::DRAW_FRAMEBUFFER`,
    /// `glow::READ_FRAMEBUFFER`.  Value `None` means "default FBO
    /// (0)"; value `Some(None)` means "no shadow yet, must re-issue".
    pub bound_framebuffer: HashMap<u32, Option<u32>>,
    /// Last-bound renderbuffer id (glBindRenderbuffer only has one
    /// target, RENDERBUFFER).
    pub bound_renderbuffer: Option<Option<u32>>,
    /// Which vertex-attribute array indices are enabled.  Keyed by
    /// attribute index (0..=MAX_VERTEX_ATTRIBS-1); presence = enabled.
    pub enabled_vertex_attribs: HashSet<u32>,
    /// Shadow for `glVertexAttribPointer`: keyed by (index), value is
    /// a fingerprint of (size, type, normalized, stride, offset) so a
    /// repeat call with identical layout skips the driver round-trip.
    /// Used by Cocos Creator 2.x's WebGL 1 path without
    /// OES_vertex_array_object — the hottest driver call in that
    /// pipeline when games draw thousands of sprites per frame.
    pub vertex_attrib_pointer_fp: HashMap<u32, VertexAttribPointerFp>,
    /// `glVertexAttribDivisor(index, divisor)` shadow.  ANGLE / WebGL 2
    /// games instancing sprites update this per attribute.
    pub vertex_attrib_divisor: HashMap<u32, u32>,
}

/// Fingerprint of the arguments to `glVertexAttribPointer`.  Kept
/// small and `Eq` so equality check inside `update_vertex_attrib_pointer`
/// is just a 16-byte comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VertexAttribPointerFp {
    pub size: i32,
    pub type_: u32,
    pub normalized: bool,
    pub stride: i32,
    pub offset: i32,
    /// VAO this pointer belongs to (0 = default VAO).  Necessary
    /// because VAOs capture vertex attrib state — a repeat call
    /// after `bindVertexArray` change must NOT be deduped.
    pub vao: u32,
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

            enabled_caps: std::collections::HashSet::new(),
            disabled_caps: std::collections::HashSet::new(),
            blend_factors: None,
            blend_equation: None,
            blend_color: None,
            depth_func: None,
            depth_mask: None,
            depth_range: None,
            cull_face: None,
            front_face: None,
            line_width: None,
            polygon_offset: None,
            unpack_alignment: None,
            pack_alignment: None,
            bound_vao: None,
            uniform_cache: HashMap::new(),
            bound_framebuffer: HashMap::new(),
            bound_renderbuffer: None,
            enabled_vertex_attribs: HashSet::new(),
            vertex_attrib_pointer_fp: HashMap::new(),
            vertex_attrib_divisor: HashMap::new(),
        }
    }
}

impl CanvasGLState {
    /// Forget every piece of shadow state that external code (Skia's
    /// text-atlas upload, DrawingBuffer blit, any non-WebGL GL caller)
    /// might have mutated during its batch.  The next WebGL call MUST
    /// re-issue rather than trust stale shadow.
    ///
    /// Rationale: Skia's draw pipeline binds programs / VAOs / FBOs /
    /// blend state / scissor / stencil / viewport / textures without
    /// going through our handlers — the same machinery backing the
    /// dedup decisions in `state_tracker.rs`.  If we kept the shadow
    /// as-is, the next Cocos-style WebGL frame might issue a
    /// `bindBuffer(ARRAY_BUFFER, 42)` that matches the last-known
    /// shadow (pre-Skia) and get silently skipped — painting with
    /// whatever buffer Skia left bound.  Bugs from this class are
    /// subtle and hard to reproduce, so the safer move is to wipe the
    /// shadow and pay one extra rebind per affected state next frame.
    ///
    /// We intentionally preserve `draws_to_default_fbo` because the
    /// render-thread caller restores the default FBO binding itself
    /// around Skia batches — leaving the flag intact is consistent
    /// with that external invariant.  If that assumption ever breaks,
    /// set the flag here too.
    /// Invalidate the dedup shadow back to "unknown".  See
    /// [`Self::invalidate_after_external_gl_use`] for rationale.
    ///
    /// This base impl only clears the P11-expansion fields; the main
    /// `invalidate_after_external_gl_use` (below) invokes it after
    /// clearing the older fields.
    fn invalidate_p11_shadow(&mut self) {
        self.bound_framebuffer.clear();
        self.bound_renderbuffer = None;
        self.enabled_vertex_attribs.clear();
        self.vertex_attrib_pointer_fp.clear();
        self.vertex_attrib_divisor.clear();
    }

    pub fn invalidate_after_external_gl_use(&mut self) {
        self.current_program = None;
        self.viewport = None;
        self.bound_texture_2d.clear();
        self.bound_array_buffer = None;
        self.bound_element_array_buffer = None;
        self.active_texture_unit = None;
        // Scissor: Skia typically leaves it disabled after its draw
        // batch completes.  Reset to the conservative "don't know
        // the rect" enabled state so the next `glScissor` call
        // re-issues; damage tracking falls back to viewport, which
        // over-reports but never under-reports.
        self.scissor = ScissorState::EnabledUnknownRect;
        self.last_scissor_rect = None;
        // Colour-mask defaults to "all channels writable" on the GL
        // initial state — but Skia may have changed it mid-batch.
        // Returning to the conservative known-nothing sentinel forces
        // the next `glColorMask` call to re-issue.
        self.color_mask = (true, true, true, true);
        self.enabled_caps.clear();
        self.disabled_caps.clear();
        self.blend_factors = None;
        self.blend_equation = None;
        self.blend_color = None;
        self.depth_func = None;
        self.depth_mask = None;
        self.depth_range = None;
        self.cull_face = None;
        self.front_face = None;
        self.line_width = None;
        self.polygon_offset = None;
        self.unpack_alignment = None;
        self.pack_alignment = None;
        self.bound_vao = None;
        // `uniform_cache` survives because uniforms are scoped to a
        // program, and the program itself is re-bound on the next GL
        // draw.  Clearing it would only save memory, not correctness.
        self.uniform_cache.clear();
        // Clear the P11 dedup fields too.
        self.invalidate_p11_shadow();
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

/// WebGL 2 Vertex Array Object.  The owner canvas is tracked so that
/// destroying a canvas also sweeps its VAOs — VAOs are not shared in
/// the EGL share-group model WebGL uses.
#[derive(Clone, Debug)]
pub(crate) struct VaoMeta {
    pub gl_handle: Option<NativeVertexArray>,
    #[allow(dead_code)]
    pub owner_canvas: Option<CanvasId>,
    pub deleted: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct SamplerMeta {
    pub gl_handle: Option<NativeSampler>,
    #[allow(dead_code)]
    pub owner_canvas: Option<CanvasId>,
    pub deleted: bool,
}

/// Fence sync.  Unlike other GL objects, these are owned by a specific
/// GL context (no sharing across contexts); we note the owner canvas so
/// `ClientWaitSync` can rebind before polling.
#[derive(Clone, Debug)]
pub(crate) struct SyncMeta {
    pub gl_handle: Option<NativeFence>,
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
