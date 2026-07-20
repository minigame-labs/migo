use std::sync::Arc;

use crossbeam_channel::Sender;
use smallvec::SmallVec;
use tokio::sync::oneshot;

pub use crate::protocol::color::Color;

use crate::error::{EngineError, ErrorCode};
use crate::protocol::FramePacket;
use crate::surface::{PixelRatio, SurfaceGeneration, SurfaceLease, SurfaceReleaseDisposition};

pub type CanvasId = u32;
pub type ImageId = u32;
pub type ProgramId = u32;
pub type ShaderId = u32;
pub type BufferId = u32;
pub type TextureId = u32;
pub type FramebufferId = u32;
pub type RenderbufferId = u32;
pub type Context2DId = u32;
pub type UniformI32Values = SmallVec<[i32; 16]>;
pub type UniformF32Values = SmallVec<[f32; 16]>;
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

/// Reply channel for a synchronous render-thread op.
///
/// The enum lives inside command variants such as
/// [`Canvas2DCmd::MeasureText`], and the render thread is expected to
/// call [`Self::send`] / [`Self::ok`] / [`Self::err`] exactly once.
///
/// ## Drop-safety contract
///
/// The inner sender is wrapped in an `Option` so the [`Drop`] impl can
/// detect the "handler forgot to reply" case and deliver
/// [`ErrorCode::Internal`] with a diagnostic detail — instead of letting
/// the sender silently drop, which would surface on the caller as the
/// extremely misleading "channel disconnected" (`ErrorCode::Disconnected`).
///
/// This is the protocol-level mitigation for P0-1 in the rendering
/// audit: sync-reply variants and fire-and-forget variants share the
/// same `Canvas2DCmd` enum, so it is all too easy to mis-match a `&cmd`
/// borrow with a reply sender that needs to be moved.  Making the
/// sender drop-safe turns every such bug from "silent render stall" into
/// "observable error in debug logs + JS op returning a proper engine
/// error".  Call sites that intentionally discard the response should
/// call [`Self::forget`] to suppress the drop-diagnostic.
#[must_use = "dropping a RenderCmdResp without sending a reply leaks the request"]
#[derive(Debug)]
pub enum RenderCmdResp<T> {
    Async(Option<oneshot::Sender<RenderResult<T>>>),
    Sync(Option<SyncResp<T>>),
}

impl<T> RenderCmdResp<T> {
    /// Wrap a sync reply sender.
    #[inline]
    pub fn from_sync(tx: SyncResp<T>) -> Self {
        Self::Sync(Some(tx))
    }

    /// Wrap an async (oneshot) reply sender.
    #[inline]
    pub fn from_async(tx: oneshot::Sender<RenderResult<T>>) -> Self {
        Self::Async(Some(tx))
    }

    /// Consume the responder and send a result on the underlying channel.
    ///
    /// Takes the sender out of the `Option` so the [`Drop`] impl below
    /// sees an empty slot and does not emit the "handler forgot to reply"
    /// diagnostic.
    #[inline]
    pub fn send(mut self, v: RenderResult<T>) {
        match &mut self {
            RenderCmdResp::Async(slot) => {
                if let Some(tx) = slot.take() {
                    let _ = tx.send(v);
                }
            }
            RenderCmdResp::Sync(slot) => {
                if let Some(tx) = slot.take() {
                    let _ = tx.send(v);
                }
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

    /// Explicitly discard the responder without triggering the
    /// drop-diagnostic.  Only valid at code paths that genuinely don't
    /// produce a reply (e.g. render thread shutdown where the caller has
    /// already been told of the failure through another channel).
    #[inline]
    pub fn forget(mut self) {
        match &mut self {
            RenderCmdResp::Async(slot) => drop(slot.take()),
            RenderCmdResp::Sync(slot) => drop(slot.take()),
        }
    }
}

/// When a `RenderCmdResp` is dropped without [`RenderCmdResp::send`] /
/// `ok` / `err` being called, surface a structured
/// `ErrorCode::Internal` on the caller so the symptom is observable in
/// logs and in JS.  Previously the sender would simply drop and the
/// receiver would observe `RecvError::Disconnected`, reported to the
/// user as `ErrorCode::Disconnected` — the same error shape as a real
/// render-thread crash, which masked genuine connectivity failures.
impl<T> Drop for RenderCmdResp<T> {
    fn drop(&mut self) {
        let err = || {
            EngineError::new(ErrorCode::Internal).with_detail(
                "render op responder dropped without sending a reply (\
                 likely a handler forgot to call resp.ok/err — upgraded \
                 from silent `channel disconnected`)"
                    .to_string(),
            )
        };
        match self {
            RenderCmdResp::Async(slot) => {
                if let Some(tx) = slot.take() {
                    let _ = tx.send(Err(err()));
                }
            }
            RenderCmdResp::Sync(slot) => {
                if let Some(tx) = slot.take() {
                    let _ = tx.send(Err(err()));
                }
            }
        }
    }
}

#[cfg(test)]
mod render_cmd_resp_drop_tests {
    use super::*;
    use crossbeam_channel::bounded;

    /// P2-6: confirm the Drop impl fires on a dropped responder
    /// and surfaces an `ErrorCode::Internal` instead of the
    /// previous silent `channel disconnected`.  This is the
    /// core safety net for bugs in the same class as the
    /// `canvas2d measure_text failed: channel disconnected`
    /// incident — if a handler forgets to reply, the caller
    /// always sees a diagnostic instead of a generic timeout.
    #[test]
    fn dropped_sync_responder_reports_internal_error() {
        let (tx, rx) = bounded::<RenderResult<u32>>(1);
        let resp = RenderCmdResp::<u32>::from_sync(tx);
        drop(resp);
        let delivered = rx.recv().expect("Drop impl should have sent a result");
        let err = delivered.expect_err("dropped responder must produce Err");
        assert_eq!(err.code, ErrorCode::Internal);
        assert!(
            err.to_string()
                .contains("responder dropped without sending"),
            "unexpected error text: {err}"
        );
    }

    #[test]
    fn explicit_send_suppresses_drop_error() {
        let (tx, rx) = bounded::<RenderResult<u32>>(1);
        let resp = RenderCmdResp::<u32>::from_sync(tx);
        resp.ok(42);
        let delivered = rx.recv().unwrap().unwrap();
        assert_eq!(delivered, 42);
    }

    #[test]
    fn forget_suppresses_drop_error_but_sends_nothing() {
        let (tx, rx) = bounded::<RenderResult<u32>>(1);
        let resp = RenderCmdResp::<u32>::from_sync(tx);
        resp.forget();
        assert!(rx.try_recv().is_err(), "forget() must not emit any Result");
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
    /// Returns the canonical font family key (used in CSS font strings).
    LoadFont {
        family: String,
        aliases: Arc<Vec<String>>,
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
    SurfaceDestroyed {
        generation: SurfaceGeneration,
    },

    /// Must-deliver native-lifetime control request.
    ///
    /// Production sends travel on `SurfaceControl`'s dedicated stream, outside
    /// the bounded draw queue.  `diagnostic` is optional and must never be used
    /// as the public release completion boundary.
    ReleaseOnscreen {
        generation: SurfaceGeneration,
        diagnostic: Option<SyncResp<SurfaceReleaseDisposition>>,
    },

    /// Trim the process-global text texture cache under OS memory
    /// pressure.  Routed to the render thread (rather than trimmed
    /// inline on the host like `io::image_cache`) because the cache
    /// holds GL textures whose `glDeleteTextures` requires a current
    /// EGL context — only the render thread has one.  `level` is the
    /// raw Android `onTrimMemory` integer; the render thread maps it
    /// via `TrimLevel::from_android`.  Best-effort: classified as a
    /// lifecycle command (drop-on-full is acceptable — the next
    /// pressure signal trims again).
    TrimTextCache {
        level: i32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TexImage3DSource {
    None,
    Bytes(std::sync::Arc<Vec<u8>>),
    BufferOffset(u32),
}

impl TexImage3DSource {
    #[inline]
    fn approx_deep_size_bytes(&self) -> usize {
        match self {
            Self::Bytes(bytes) => bytes.capacity(),
            Self::None | Self::BufferOffset(_) => 0,
        }
    }
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

    /// Fire-and-forget offscreen canvas creation.  JS allocates the
    /// `id` itself from a private high-range counter so the render
    /// thread doesn't have to round-trip a sync response.
    ///
    /// On the goldfish emulator each EGL pbuffer context creation
    /// costs 30–100ms; cocos's font-atlas / label-cache code path
    /// fires a burst of ~50 of these when opening a popup, which
    /// would otherwise push the parent sync op past its 1s timeout
    /// and crash the V8 host with `[Timeout] create_offscreen_canvas`.
    /// Subsequent ops targeting this canvas (`getContext`, draw)
    /// queue on the same FIFO so ordering is preserved.
    RegisterOffscreen {
        id: CanvasId,
        width: u32,
        height: u32,
    },

    DestroyCanvas {
        id: CanvasId,
        resp: RenderCmdResp<()>,
    },

    RecreateOnscreen {
        lease: SurfaceLease,
        /// Transactional DPR update. The backend commits this only after the
        /// Surface installation succeeds; `None` preserves the current value.
        pixel_ratio: Option<PixelRatio>,
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

    // Image resources (owned by render thread).
    //
    // Note: id allocation no longer round-trips through the render
    // thread — both JS and render-thread callers pull from
    // `shared::image_id::next_image_id()` instead.  The id is sent
    // pre-allocated on the `LoadImage` below.
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

    /// Batch variant of [`CanvasCmd::DestroyImage`]: destroy many shared images
    /// in a single command. Bulk teardown paths (clearImageCache, session
    /// restart) can enqueue potentially hundreds of ids; sending them one by one
    /// as must-deliver Sync-class commands can block the producer up to the send
    /// deadline *per id*. Batching bounds that to a single bounded-blocking send
    /// while preserving must-deliver semantics.
    DestroyImages {
        image_ids: Vec<ImageId>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderType {
    Vertex,
    Fragment,
}

/// Upper bound on the stack size of a single [`GLCmd`] variant.
///
/// Keeping the enum under this cap matters because:
///
/// 1. The frame collector allocates `Vec<GLCmd>` segments that
///    use this size as the per-element slot; doubling it halves
///    cache-line density of the playback walk.
/// 2. `approx_deep_size_bytes()` starts from `size_of::<GLCmd>()`
///    and adds heap payloads on top; an inflated base inflates
///    every accounting call - including the auto-flush guard.
///
/// Current number chosen from the actual Rust layout as of this
/// commit (largest variant is `TexImage3D` at ~136 B on 64-bit
/// due to the 3D-texture params).  Raise it deliberately when a
/// new variant needs room; the assertion below flags the next
/// accidental bloat at compile time.
pub const GLCMD_MAX_SIZE_BYTES: usize = 192;

// Compile-time check that new variants do not silently grow the
// enum past the budget.
const _: () = {
    if std::mem::size_of::<GLCmd>() > GLCMD_MAX_SIZE_BYTES {
        panic!(
            "GLCmd grew past GLCMD_MAX_SIZE_BYTES; consider boxing \
             the new payload or raising the cap deliberately"
        );
    }
};

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

    /// Debug trigger for `WEBGL_lose_context.loseContext()`: arm a one-shot
    /// simulated GPU reset on the render thread so the real context-loss
    /// recovery pipeline can be exercised on demand.
    DebugLoseContext {
        canvas_id: CanvasId,
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

    // Fire-and-forget: binds an attribute index for a program (takes effect on
    // the next LinkProgram). Program-context op like LinkProgram — no canvas_id.
    BindAttribLocation {
        program_id: ProgramId,
        index: u32,
        name: String,
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
        value: UniformF32Values,
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
    /// `glTexImage2D(target, level, internalformat, ..., image)` where
    /// `image` is a previously loaded shared image (uploaded via
    /// `CanvasCmd::LoadImage`).  Avoids the round-trip of CPU-side
    /// RGBA bytes from JS land back to the render thread by copying
    /// straight from the existing GL texture into the destination.
    /// The render thread issues a GPU-side copy (FBO + glCopyTexImage2D).
    /// The destination is whichever texture is currently bound to
    /// `target` on the canvas's context — same convention as
    /// [`Self::TexImage2D`].
    TexImage2DFromShared {
        canvas_id: CanvasId,
        target: u32,
        level: i32,
        internalformat: i32,
        format: u32,
        type_: u32,
        /// Identifier of the source shared image in `ImageStore`.
        source_shared_id: u32,
        /// Logical image size; for atlased entries this is the
        /// sub-rect size, not the atlas page dims.
        src_width: i32,
        src_height: i32,
    },
    /// Zero-readback Canvas2D->WebGL upload, sibling of
    /// [`Self::TexImage2DFromShared`].  Source is a snapshot
    /// texture allocated by [`Canvas2DCmd::GetImageDataSnapshot`];
    /// destination is whichever texture is currently bound to
    /// `target` on `canvas_id`.  Render thread issues an FBO+
    /// `glCopyTexImage2D` GPU copy — same primitive that powers the
    /// shared-image path.  Snapshot is NOT consumed (refcount-free
    /// for now): per-frame drain releases all live snapshots after
    /// present, so a single getImageData→texImage2D pair within a
    /// frame is the supported lifetime.
    TexImage2DFromSnapshot {
        canvas_id: CanvasId,
        target: u32,
        level: i32,
        internalformat: i32,
        format: u32,
        type_: u32,
        snapshot_id: u32,
    },
    /// Text texture cache hit path.  When the JS-side pattern
    /// recognizer in `frame_collector` matches the cocos
    /// `(state setters → fillText → texImage2D(canvas))` shape AND
    /// the `(text, font, size, color, ...)` tuple is already
    /// present in `shared::text_texture_cache::global_cache()`, JS
    /// suppresses the offscreen fillText + snapshot pipeline
    /// entirely and emits this command instead.  Render thread
    /// re-acquires the cache lock, copies the cached source texture
    /// into the destination texture bound to `target` on
    /// `canvas_id` (single GPU→GPU copy, no Skia paint, no blit
    /// from Canvas2D FBO), and unpins the entry.
    ///
    /// `key` is boxed because `TextCacheKey` carries two `String`s;
    /// the unboxed variant would inflate every `GLCmd` instance and
    /// every `CanvasBatchPayload` cmd vec across the channel.
    TexImage2DFromTextCache {
        canvas_id: CanvasId,
        target: u32,
        level: i32,
        internalformat: i32,
        key: Box<crate::text_texture_cache::TextCacheKey>,
    },
    /// Direct GPU->GPU upload from a Canvas2D's framebuffer to a WebGL
    /// texture, no JS-visible snapshot id, no readback.  Optimises the
    /// cocos `gl.texImage2D(target, ..., HTMLCanvasElement)` pattern --
    /// previously routed through `sourceToRawRgba` -> getImageData ->
    /// lazy readback (~50ms V8 stall per call, ~20 calls per popup
    /// open).  The render thread does FBO blit + glCopyTexImage2D in
    /// one shot, freeing the temp source texture immediately.
    TexImage2DFromCanvas2D {
        canvas_id: CanvasId, // GL canvas (where dst tex is bound)
        target: u32,
        level: i32,
        internalformat: i32,
        canvas_2d_id: CanvasId, // 2D canvas (source content)
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    /// Sub-region variant of `TexImage2DFromCanvas2D`: copies the 2D
    /// canvas's content into a sub-rect of an already-allocated texture.
    /// Required for cocos's text-atlas pattern (allocate atlas once,
    /// stream glyph cells in via texSubImage2D).
    TexSubImage2DFromCanvas2D {
        canvas_id: CanvasId,
        target: u32,
        level: i32,
        xoffset: i32,
        yoffset: i32,
        canvas_2d_id: CanvasId,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    /// Sibling of `TexImage2DFromSnapshot` for `texSubImage2D` -- copies
    /// the snapshot texture into a sub-region of the destination texture
    /// currently bound to `target` on `canvas_id` via FBO +
    /// `glCopyTexSubImage2D`.  Required for cocos's text-atlas pattern,
    /// which pre-allocates the atlas with `texImage2D` and then updates
    /// individual glyph cells via `texSubImage2D`.  Width/height come
    /// from the snapshot itself; JS only routes here when the caller's
    /// (width, height) matches the snapshot's (or it's the 7-arg form
    /// without explicit dims), so partial copies fall back to bytes.
    TexSubImage2DFromSnapshot {
        canvas_id: CanvasId,
        target: u32,
        level: i32,
        xoffset: i32,
        yoffset: i32,
        format: u32,
        type_: u32,
        snapshot_id: u32,
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
        value: UniformI32Values,
    },
    Uniform1fv {
        canvas_id: CanvasId,
        location: Option<u32>,
        value: UniformF32Values,
    },
    Uniform2iv {
        canvas_id: CanvasId,
        location: Option<u32>,
        value: UniformI32Values,
    },
    Uniform2fv {
        canvas_id: CanvasId,
        location: Option<u32>,
        value: UniformF32Values,
    },
    Uniform3iv {
        canvas_id: CanvasId,
        location: Option<u32>,
        value: UniformI32Values,
    },
    Uniform3fv {
        canvas_id: CanvasId,
        location: Option<u32>,
        value: UniformF32Values,
    },
    Uniform4iv {
        canvas_id: CanvasId,
        location: Option<u32>,
        value: UniformI32Values,
    },
    Uniform4fv {
        canvas_id: CanvasId,
        location: Option<u32>,
        value: UniformF32Values,
    },
    UniformMatrix2fv {
        canvas_id: CanvasId,
        location: Option<u32>,
        transpose: bool,
        value: UniformF32Values,
    },
    UniformMatrix4fv {
        canvas_id: CanvasId,
        location: Option<u32>,
        transpose: bool,
        value: UniformF32Values,
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
    DeleteBuffer {
        buffer_id: BufferId,
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

    // ---- WebGL 2 Query objects ----
    //
    // Queries are non-blocking result probes for things like
    // `ANY_SAMPLES_PASSED` (occlusion) and `TIME_ELAPSED_EXT`.  The
    // engine allocates a client id synchronously (see
    // `op_alloc_gl_resource_id`) and creates the underlying GL
    // object on the render thread.
    CreateQuery {
        canvas_id: CanvasId,
        client_id: u32,
    },
    DeleteQuery {
        query: u32,
    },
    BeginQuery {
        canvas_id: CanvasId,
        target: u32,
        query: u32,
    },
    EndQuery {
        canvas_id: CanvasId,
        target: u32,
    },
    /// Synchronous fetch of `GL_QUERY_RESULT` /
    /// `GL_QUERY_RESULT_AVAILABLE`.  Resp carries the u32 result.
    GetQueryParameter {
        query: u32,
        pname: u32,
        resp: RenderCmdResp<u32>,
    },

    // ---- WebGL 2 Transform Feedback ----
    //
    // Transform feedback is a container for captured vertex-shader
    // output during draw calls.  Enough of the surface is exposed
    // that Cocos Creator 3.x particle systems light up; the full
    // API (getTransformFeedbackVarying, pause/resume) follows the
    // same pattern and can be added incrementally.
    CreateTransformFeedback {
        canvas_id: CanvasId,
        client_id: u32,
    },
    DeleteTransformFeedback {
        tf: u32,
    },
    BindTransformFeedback {
        canvas_id: CanvasId,
        target: u32,
        tf: Option<u32>,
    },
    BeginTransformFeedback {
        canvas_id: CanvasId,
        primitive_mode: u32,
    },
    EndTransformFeedback {
        canvas_id: CanvasId,
    },
    PauseTransformFeedback {
        canvas_id: CanvasId,
    },
    ResumeTransformFeedback {
        canvas_id: CanvasId,
    },
    /// `transformFeedbackVaryings(program, varyings, buffer_mode)`.
    /// `varyings` are the shader output names; `buffer_mode` is one
    /// of `INTERLEAVED_ATTRIBS` / `SEPARATE_ATTRIBS`.
    TransformFeedbackVaryings {
        canvas_id: CanvasId,
        program: ProgramId,
        varyings: Vec<String>,
        buffer_mode: u32,
    },
    /// Synchronous fetch of linked transform-feedback varying metadata.
    /// Returns `Some((name, size, type))` for valid indices.
    GetTransformFeedbackVarying {
        program: ProgramId,
        index: u32,
        resp: RenderCmdResp<Option<(String, i32, u32)>>,
    },

    // ---- WebGL 2 3D textures ----
    //
    // Only the data-upload variants are wired here; allocation via
    // `texStorage3D` already exists as part of the earlier WebGL 2
    // tranche and is reused unchanged.
    TexImage3D {
        canvas_id: CanvasId,
        target: u32,
        level: i32,
        internal_format: i32,
        width: i32,
        height: i32,
        depth: i32,
        border: i32,
        format: u32,
        ty: u32,
        /// RGBA / Luminance / etc. byte stream - `None` reserves
        /// storage without an upload, matching the WebGL 2
        /// "size-only" overload.
        data: TexImage3DSource,
    },
    TexSubImage3D {
        canvas_id: CanvasId,
        target: u32,
        level: i32,
        xoffset: i32,
        yoffset: i32,
        zoffset: i32,
        width: i32,
        height: i32,
        depth: i32,
        format: u32,
        ty: u32,
        data: TexImage3DSource,
    },
    TexStorage3D {
        canvas_id: CanvasId,
        target: u32,
        levels: i32,
        internal_format: u32,
        width: i32,
        height: i32,
        depth: i32,
    },
}

/// Text horizontal alignment for fillText/strokeText.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextAlign {
    #[default]
    Start,
    End,
    Left,
    Right,
    Center,
}

/// Text vertical baseline for fillText/strokeText.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
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
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize,
)]
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
    /// Init the Skia surface for the target canvas.  Fire-and-forget:
    /// the render thread processes commands FIFO, so this op completes
    /// before any subsequent Canvas2D command on the same canvas runs.
    /// Removed the sync reply channel because its only payload was the
    /// caller's own `canvas_id` — pure round-trip stall (was ~7–17 ms
    /// per call × dozens per shop-scene open on Mali).
    CreateContext2D,

    /// Resize the backing pbuffer + SkSurface in-band with the rest of
    /// the Canvas2D command stream.  Mirrors `CanvasCmd::ResizeCanvas`
    /// but routes through the frame collector so it interleaves with
    /// `FillText` / `TexImage2DFromCanvas2D` in the order JS issued
    /// them.  Required for cocos's text-label pattern where a single
    /// pooled canvas is repeatedly resized and re-filled within a
    /// frame; without this, the resize side-channel races ahead of
    /// the buffered draws and the texture upload sees the wrong-size
    /// surface (random-blank label symptom on arm64).
    ResizeCanvas {
        w: Option<u32>,
        h: Option<u32>,
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

    /// Fire-and-forget snapshot variant for the cocos hot path.
    /// JS allocates the `snapshot_id` from a process-local counter
    /// so the call never blocks — the capture rides the next
    /// FramePacket alongside the prior canvas2D draws and the
    /// downstream `TexImage2DFromSnapshot` GL op, all in command
    /// order.  On capture failure (FBO incomplete, pool full) the
    /// id is silently absent from the pool; the consuming
    /// `TexImage2DFromSnapshot` then warns.  JS-side cap (
    /// `MAX_LIVE_CANVAS2D_SNAPSHOTS_JS`) keeps the per-frame count
    /// well under the render-side pool cap so the failure path is
    /// never reached in normal operation.
    CaptureSnapshot {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        snapshot_id: u32,
        /// Miss-path record for the text texture cache.  When the
        /// JS-side pattern recognizer matched the cocos
        /// `fillText → texImage2D(canvas)` shape but the cache
        /// missed, this command both captures the snapshot (so the
        /// downstream `TexImage2DFromSnapshot` still works) **and**
        /// instructs the render thread to register the snapshot's
        /// resulting texture under this key.  Subsequent fillTexts
        /// with the same key hit `TexImage2DFromTextCache`.
        ///
        /// `None` means "no cache record" — the legacy path; the
        /// snapshot is consumed by `TexImage2DFromSnapshot` and
        /// drained at frame end like always.
        cache_key: Option<Box<crate::text_texture_cache::TextCacheKey>>,
    },

    /// Force a synchronous CPU readback of a previously created
    /// snapshot texture.  Backs `migo._force_readback(imageData)`
    /// for the rare cocos-incompatible game that actually inspects
    /// `ImageData.data` bytes after `getImageData`.
    ///
    /// Returns `Vec<u8>` of length `width * height * 4` (RGBA8
    /// unpremul, top-left origin) on success, empty on failure.
    /// Snapshot orientation is set up so the bytes match what the
    /// legacy `GetImageData` path would have returned for the same
    /// region.
    ReadSnapshotPixels {
        snapshot_id: u32,
        resp: RenderCmdResp<Vec<u8>>,
    },

    /// Batch draw multiple images for better performance
    /// Each entry is (image_id, sx, sy, sw, sh, dx, dy, dw, dh)
    DrawImageBatch {
        draws: Vec<DrawImageEntry>,
    },
}

impl Canvas2DCmd {
    /// Whether executing this command consults the shared text shaping and
    /// font context. This match is deliberately exhaustive so every future
    /// command must make an explicit lock decision when it is introduced.
    #[inline]
    pub fn requires_text_context(&self) -> bool {
        match self {
            Self::FillText { .. } | Self::StrokeText { .. } | Self::MeasureText { .. } => true,

            Self::CreateContext2D
            | Self::ResizeCanvas { .. }
            | Self::BeginPath
            | Self::ClosePath
            | Self::MoveTo { .. }
            | Self::LineTo { .. }
            | Self::QuadraticCurveTo { .. }
            | Self::BezierCurveTo { .. }
            | Self::Arc { .. }
            | Self::ArcTo { .. }
            | Self::Rect { .. }
            | Self::Ellipse { .. }
            | Self::Fill
            | Self::Stroke
            | Self::Clip
            | Self::FillRect { .. }
            | Self::StrokeRect { .. }
            | Self::ClearRect { .. }
            | Self::SetFillStyle { .. }
            | Self::SetStrokeStyle { .. }
            | Self::SetLineWidth { .. }
            | Self::SetLineCap { .. }
            | Self::SetLineJoin { .. }
            | Self::SetMiterLimit { .. }
            | Self::SetGlobalAlpha { .. }
            | Self::SetCompositeOperation { .. }
            | Self::SetLineDash { .. }
            | Self::SetLineDashOffset { .. }
            | Self::SetShadowBlur { .. }
            | Self::SetShadowColor { .. }
            | Self::SetShadowOffsetX { .. }
            | Self::SetShadowOffsetY { .. }
            | Self::SetFillStyleGradient { .. }
            | Self::SetStrokeStyleGradient { .. }
            | Self::SetFillStylePattern { .. }
            | Self::SetStrokeStylePattern { .. }
            | Self::SetFont { .. }
            | Self::SetTextAlign { .. }
            | Self::SetTextBaseline { .. }
            | Self::SetTextDirection { .. }
            | Self::Save
            | Self::Restore
            | Self::SetTransform { .. }
            | Self::ResetTransform
            | Self::Translate { .. }
            | Self::Rotate { .. }
            | Self::Scale { .. }
            | Self::DrawImage { .. }
            | Self::GetImageData { .. }
            | Self::CaptureSnapshot { .. }
            | Self::ReadSnapshotPixels { .. }
            | Self::DrawImageBatch { .. } => false,
        }
    }
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
            | GLCmd::FenceSync { canvas_id, .. }
            | GLCmd::CreateQuery { canvas_id, .. }
            | GLCmd::BeginQuery { canvas_id, .. }
            | GLCmd::EndQuery { canvas_id, .. }
            | GLCmd::CreateTransformFeedback { canvas_id, .. }
            | GLCmd::BindTransformFeedback { canvas_id, .. }
            | GLCmd::BeginTransformFeedback { canvas_id, .. }
            | GLCmd::EndTransformFeedback { canvas_id, .. }
            | GLCmd::PauseTransformFeedback { canvas_id, .. }
            | GLCmd::ResumeTransformFeedback { canvas_id, .. }
            | GLCmd::TransformFeedbackVaryings { canvas_id, .. }
            | GLCmd::TexImage3D { canvas_id, .. }
            | GLCmd::TexSubImage3D { canvas_id, .. }
            | GLCmd::TexStorage3D { canvas_id, .. } => Some(*canvas_id),

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
            | GLCmd::GetUniformBlockIndex { name, .. }
            | GLCmd::BindAttribLocation { name, .. } => name.capacity(),

            // Buffer uploads — the dominant budget item for 3D games.
            // `BufferData.data` is optional (spec allows passing
            // `null` to reserve without upload).
            GLCmd::BufferData { data, .. } => data.as_ref().map_or(0, |v| v.capacity()),
            GLCmd::BufferSubData { data, .. } => data.capacity(),

            // Texture uploads (RGBA or compressed block).  `TexImage2D`
            // is optional data (reservation vs upload); `TexSubImage2D`
            // is always `Arc<Vec<u8>>` with a concrete payload.
            GLCmd::TexImage2D { data, .. } => data.as_ref().map_or(0, |arc| arc.capacity()),
            GLCmd::TexSubImage2D { data, .. } => data.capacity(),
            GLCmd::CompressedTexImage2D { data, .. } => data.capacity(),
            GLCmd::CompressedTexSubImage2D { data, .. } => data.capacity(),

            // Uniform array uploads — scalar per element, but a
            // `uniform4fv(bones[100])` is 400 floats = 1.6 KB.
            GLCmd::Uniform1iv { value, .. }
            | GLCmd::Uniform2iv { value, .. }
            | GLCmd::Uniform3iv { value, .. }
            | GLCmd::Uniform4iv { value, .. } => {
                usize::from(value.spilled()) * value.capacity() * std::mem::size_of::<i32>()
            }
            GLCmd::Uniform1fv { value, .. }
            | GLCmd::Uniform2fv { value, .. }
            | GLCmd::Uniform3fv { value, .. }
            | GLCmd::Uniform4fv { value, .. }
            | GLCmd::UniformMatrix2fv { value, .. }
            | GLCmd::UniformMatrix3fv { value, .. }
            | GLCmd::UniformMatrix4fv { value, .. } => {
                usize::from(value.spilled()) * value.capacity() * std::mem::size_of::<f32>()
            }

            // WebGL 2 framebuffer metadata arrays.
            GLCmd::InvalidateFramebuffer { attachments, .. } => {
                attachments.capacity() * std::mem::size_of::<u32>()
            }
            GLCmd::DrawBuffers { buffers, .. } => buffers.capacity() * std::mem::size_of::<u32>(),

            // WebGL 2 transform feedback varying names.
            GLCmd::TransformFeedbackVaryings { varyings, .. } => {
                varyings.iter().map(|v| v.capacity()).sum()
            }

            // WebGL 2 3D texture payloads.
            GLCmd::TexImage3D { data, .. } | GLCmd::TexSubImage3D { data, .. } => {
                data.approx_deep_size_bytes()
            }

            // All other variants are pure scalars / Copy payloads -
            // the enum stack size already accounts for them.
            _ => 0,
        }
    }
}

impl Canvas2DCmd {
    /// Feed each image id this command references into `sink`.
    /// Used by the render thread (F-1) to pin image entries in
    /// the `ImageStore` for as long as a `FramePacket` carrying
    /// the command is in flight — a concurrent `DestroyImage`
    /// then defers the GL `glDeleteTextures` call until the
    /// Present barrier fires, eliminating the race where Skia's
    /// deferred command buffer still references the backend
    /// texture at the moment of deletion.
    ///
    /// Pass-through method (no allocation) so the caller can use
    /// it with `for_each` or accumulate into any collection.
    #[inline]
    pub fn for_each_referenced_image<F: FnMut(ImageId)>(&self, mut sink: F) {
        match self {
            Canvas2DCmd::DrawImage { image_id, .. } => sink(*image_id),
            Canvas2DCmd::DrawImageBatch { draws } => {
                for d in draws {
                    sink(d.image_id);
                }
            }
            // Other variants don't reference `image_id`.  `#[non_exhaustive]`
            // on `Canvas2DCmd` is honoured by the explicit match + catch-all.
            _ => {}
        }
    }

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
        let value = vec![0.0f32; 48].into();
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
    fn single_mat4_uniform_payload_is_inline() {
        let cmd = GLCmd::UniformMatrix4fv {
            canvas_id: CanvasId::from(1u32),
            location: Some(0),
            transpose: false,
            value: (0..16).map(|n| n as f32).collect(),
        };

        assert_eq!(
            cmd.approx_deep_size_bytes(),
            std::mem::size_of::<GLCmd>(),
            "one mat4 should fit in the command's inline uniform storage"
        );
    }

    #[test]
    fn uniform_payload_over_inline_limit_counts_spilled_capacity() {
        let cmd = GLCmd::Uniform1fv {
            canvas_id: CanvasId::from(1u32),
            location: Some(0),
            value: (0..17).map(|n| n as f32).collect(),
        };

        assert!(
            cmd.approx_deep_size_bytes() >= std::mem::size_of::<GLCmd>() + 17 * 4,
            "17 floats must spill and contribute their heap capacity"
        );
    }

    #[test]
    fn gl_command_size_stays_within_existing_cache_line_budget() {
        assert!(
            std::mem::size_of::<GLCmd>() <= 144,
            "GLCmd grew to {} bytes",
            std::mem::size_of::<GLCmd>()
        );
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
        let mut touched: std::collections::HashSet<CanvasId> = std::collections::HashSet::new();
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
        let mut touched: std::collections::HashSet<CanvasId> = std::collections::HashSet::new();
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

#[cfg(test)]
mod text_context_tests {
    use super::{Canvas2DCmd, RenderCmdResp, TextAlign, TextMetrics};

    #[test]
    fn only_text_execution_commands_require_the_shared_context() {
        let fill = Canvas2DCmd::FillText {
            text: "fill".into(),
            x: 1.0,
            y: 2.0,
            max_width: 100.0,
        };
        let stroke = Canvas2DCmd::StrokeText {
            text: "stroke".into(),
            x: 1.0,
            y: 2.0,
            max_width: 100.0,
        };
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let measure = Canvas2DCmd::MeasureText {
            text: "measure".into(),
            resp: RenderCmdResp::<TextMetrics>::from_sync(tx),
        };

        assert!(fill.requires_text_context());
        assert!(stroke.requires_text_context());
        assert!(measure.requires_text_context());

        if let Canvas2DCmd::MeasureText { resp, .. } = measure {
            resp.forget();
        }
    }

    #[test]
    fn text_state_and_non_text_commands_do_not_require_the_shared_context() {
        let commands = [
            Canvas2DCmd::SetFont {
                font: "16px sans-serif".into(),
            },
            Canvas2DCmd::SetTextAlign {
                align: TextAlign::Center,
            },
            Canvas2DCmd::FillRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
            Canvas2DCmd::DrawImage {
                image_id: 7,
                sx: 0.0,
                sy: 0.0,
                sw: 1.0,
                sh: 1.0,
                dx: 0.0,
                dy: 0.0,
                dw: 1.0,
                dh: 1.0,
            },
            Canvas2DCmd::BeginPath,
            Canvas2DCmd::Save,
            Canvas2DCmd::CaptureSnapshot {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                snapshot_id: 1,
                cache_key: None,
            },
        ];

        for command in commands {
            assert!(
                !command.requires_text_context(),
                "unexpected text lock requirement for {command:?}"
            );
        }
    }
}
