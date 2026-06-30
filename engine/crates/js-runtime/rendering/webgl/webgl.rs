use std::sync::Arc;

use bytemuck::allocation::cast_vec;
use deno_core::{OpState, op2};
use tracing::{error, warn};

use crate::rendering::image::cache::IMAGE_CACHE;

use shared::{
    error::EngineError,
    js_escape::escape_for_json_string,
    op_state::CanvasOpState,
    protocol::{
        render_cmd::{GLCmd, RenderCmdResp, RenderCommand, ShaderType},
        send_gl_with_resp_sync,
    },
};

pub(crate) struct GlResourceIdAllocator {
    next_id: u32,
}

impl GlResourceIdAllocator {
    pub(crate) fn new() -> Self {
        Self { next_id: 1 }
    }

    fn alloc(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        if self.next_id == 0 {
            self.next_id = 1;
        }
        id
    }

    #[cfg(test)]
    fn with_next_for_test(next_id: u32) -> Self {
        Self {
            next_id: if next_id == 0 { 1 } else { next_id },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        path::PathBuf,
        rc::Rc,
        sync::{Arc, atomic::AtomicBool},
        time::Duration,
    };

    use deno_core::OpState;
    use tokio::sync::mpsc;

    use super::{
        GlResourceIdAllocator, bind_buffer_base_impl, bind_buffer_range_impl,
        normalize_tex_upload_3d_source,
    };
    use crate::rendering::webgl::{
        error_state::{self, WebGLErrorState, codes},
        frame_collector::UnifiedFrameCollector,
    };
    use crate::{HostJsRuntime, host_runtime::SharedMountTableRef};
    use shared::{
        channel::ThreadWakeup,
        device::gpu_caps::GpuCaps,
        op_state::{AudioSender, HostOpState, NetworkPolicy},
        protocol::render_cmd::{GLCmd, RenderCommand, TexImage3DSource},
        render_command_sender::CommandSender,
    };

    fn new_webgl_op_state() -> OpState {
        let mut state = OpState::new(None);
        state.put(UnifiedFrameCollector::new());
        state.put(WebGLErrorState::default());
        state
    }

    fn new_test_host_state() -> (HostOpState, crossbeam_channel::Receiver<RenderCommand>) {
        let (render_tx, render_rx) = CommandSender::new();
        let (audio_raw_tx, _audio_rx) = mpsc::unbounded_channel();
        let (host_tx, _host_rx) = mpsc::channel(1);

        (
            HostOpState {
                id: 1,
                app_cache_dir: PathBuf::from("/tmp/cache"),
                app_files_dir: PathBuf::from("/tmp/files"),
                code_dir: None,
                game_paths: None,
                vfs: None,
                mount_table: None,
                render_tx,
                text_measurer: None,
                audio_tx: AudioSender::new(audio_raw_tx, ThreadWakeup::new()),
                host_tx,
                device_services: None,
                raf_rx: None,
                sub_packages: Vec::new(),
                workers_path: None,
                network_policy: NetworkPolicy::default(),
                backgrounded: Arc::new(AtomicBool::new(false)),
                webgl_context_created: Arc::new(AtomicBool::new(false)),
                code_signing_enabled: false,
                gpu_caps: GpuCaps::new(),
            },
            render_rx,
        )
    }

    fn new_webgl_runtime() -> (HostJsRuntime, crossbeam_channel::Receiver<RenderCommand>) {
        let (host_state, render_rx) = new_test_host_state();
        let mount_ref: SharedMountTableRef = Rc::new(RefCell::new(None));
        let runtime = HostJsRuntime::new(
            1,
            host_state,
            Vec::new(),
            None,
            None,
            mount_ref,
            #[cfg(feature = "v8-limits")]
            Default::default(),
            #[cfg(feature = "code-signing")]
            false,
            #[cfg(feature = "code-signing")]
            None,
        );
        (runtime, render_rx)
    }

    fn spawn_tf_varying_responder(
        render_rx: crossbeam_channel::Receiver<RenderCommand>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let mut replies = 0;
            while replies < 2 {
                match render_rx
                    .recv_timeout(Duration::from_secs(1))
                    .expect("expected transform feedback varying request")
                {
                    RenderCommand::FramePacket(_) => {}
                    RenderCommand::GL(GLCmd::GetTransformFeedbackVarying {
                        program,
                        index,
                        resp,
                    }) => {
                        assert_eq!(program, 17);
                        match index {
                            0 => resp.ok(Some(("v_pos".to_string(), 1, 0x8B51))),
                            1 => resp.ok(None),
                            other => panic!("unexpected varying index {other}"),
                        }
                        replies += 1;
                    }
                    other => panic!("unexpected render command in TF varying test: {other:?}"),
                }
            }
        })
    }

    #[test]
    fn allocates_monotonic_non_zero_ids() {
        let mut alloc = GlResourceIdAllocator::new();
        assert_eq!(alloc.alloc(), 1);
        assert_eq!(alloc.alloc(), 2);
        assert_eq!(alloc.alloc(), 3);
    }

    #[test]
    fn wraps_without_returning_zero() {
        let mut alloc = GlResourceIdAllocator::with_next_for_test(u32::MAX);
        assert_eq!(alloc.alloc(), u32::MAX);
        assert_eq!(alloc.alloc(), 1);
        assert_ne!(alloc.alloc(), 0);
    }

    #[test]
    fn bind_buffer_base_rejects_transform_feedback_target_while_active() {
        let mut state = new_webgl_op_state();
        error_state::set_transform_feedback_active(&mut state, 7, true);

        bind_buffer_base_impl(&mut state, 7, 0x8C8E, 0, 9);

        assert_eq!(
            state.borrow_mut::<WebGLErrorState>().drain_one(7),
            codes::INVALID_OPERATION
        );
        assert_eq!(
            state
                .borrow::<UnifiedFrameCollector>()
                .approx_pending_bytes(),
            0,
            "validator must reject the bind before queueing GL work"
        );
    }

    #[test]
    fn bind_buffer_range_rejects_transform_feedback_target_while_active() {
        let mut state = new_webgl_op_state();
        error_state::set_transform_feedback_active(&mut state, 7, true);

        bind_buffer_range_impl(&mut state, 7, 0x8C8E, 0, 9, 0, 64);

        assert_eq!(
            state.borrow_mut::<WebGLErrorState>().drain_one(7),
            codes::INVALID_OPERATION
        );
        assert_eq!(
            state
                .borrow::<UnifiedFrameCollector>()
                .approx_pending_bytes(),
            0,
            "validator must reject the bind before queueing GL work"
        );
    }

    #[test]
    fn tex_image_3d_source_applies_src_offset_in_elements() {
        match normalize_tex_upload_3d_source(Some(&[0, 1, 2, 3, 4, 5, 6, 7]), 2, 2, None) {
            TexImage3DSource::Bytes(bytes) => assert_eq!(bytes.as_slice(), &[4, 5, 6, 7]),
            other => panic!("expected sliced byte source, got {other:?}"),
        }
    }

    #[test]
    fn tex_sub_image_3d_source_uses_pbo_offset_when_requested() {
        match normalize_tex_upload_3d_source(None, 0, 1, Some(24)) {
            TexImage3DSource::BufferOffset(offset) => assert_eq!(offset, 24),
            other => panic!("expected buffer offset source, got {other:?}"),
        }
    }

    #[test]
    fn webgl2_query_registry_matches_lifecycle_semantics() {
        let (mut runtime, _render_rx) = new_webgl_runtime();
        runtime
            .exec_script(
                "webgl2_query_registry.js",
                r#"
                const ctx = new WebGL2RenderingContext({ _rid: 7, width: 1, height: 1 }, {});
                const q = ctx.createQuery();
                if (ctx.isQuery(q) !== false) throw new Error("createQuery must not become true before first beginQuery");
                if (ctx.getQuery(0x8C2F, 0x8865) !== null) throw new Error("CURRENT_QUERY must start as null");
                ctx.beginQuery(0x8C2F, q);
                if (ctx.isQuery(q) !== true) throw new Error("beginQuery must mark query as real");
                if (ctx.getQuery(0x8C2F, 0x8865) !== q) throw new Error("CURRENT_QUERY must return the active query object");
                ctx.endQuery(0x8C2F);
                if (ctx.getQuery(0x8C2F, 0x8865) !== null) throw new Error("CURRENT_QUERY must clear after endQuery");
                ctx.deleteQuery(q);
                if (ctx.isQuery(q) !== false) throw new Error("deleteQuery must make isQuery false");
                "#,
            )
            .expect("query lifecycle script should complete");
    }

    #[test]
    fn webgl2_transform_feedback_delete_active_queues_invalid_operation() {
        let (mut runtime, _render_rx) = new_webgl_runtime();
        runtime
            .exec_script(
                "webgl2_tf_registry.js",
                r#"
                const ctx = new WebGL2RenderingContext({ _rid: 9, width: 1, height: 1 }, {});
                const tf = ctx.createTransformFeedback();
                if (ctx.isTransformFeedback(tf) !== false) throw new Error("createTransformFeedback must not become true before first bind");
                ctx.bindTransformFeedback(0x8E22, tf);
                if (ctx.isTransformFeedback(tf) !== true) throw new Error("bindTransformFeedback must mark object as real");
                ctx.beginTransformFeedback(0x0004);
                ctx.pauseTransformFeedback();
                ctx.deleteTransformFeedback(tf);
                if (ctx.getError() !== 0x0502) throw new Error("delete active transform feedback must queue INVALID_OPERATION");
                if (ctx.isTransformFeedback(tf) !== true) throw new Error("failed delete must preserve transform feedback object");
                ctx.endTransformFeedback();
                ctx.deleteTransformFeedback(tf);
                if (ctx.isTransformFeedback(tf) !== false) throw new Error("successful delete must clear transform feedback object");
                "#,
            )
            .expect("transform feedback lifecycle script should complete");
    }

    #[test]
    fn webgl2_get_transform_feedback_varying_parses_sync_result() {
        let (mut runtime, render_rx) = new_webgl_runtime();
        let responder = spawn_tf_varying_responder(render_rx);
        runtime
            .exec_script(
                "webgl2_tf_varying.js",
                r#"
                const ctx = new WebGL2RenderingContext({ _rid: 11, width: 1, height: 1 }, {});
                const program = { _id: 17, _kind: "program" };
                const info = ctx.getTransformFeedbackVarying(program, 0);
                if (!info) throw new Error("expected transform feedback varying metadata");
                if (info.name !== "v_pos") throw new Error("varying name mismatch");
                if (info.size !== 1) throw new Error("varying size mismatch");
                if (info.type !== 0x8B51) throw new Error("varying type mismatch");
                if (ctx.getTransformFeedbackVarying(program, 1) !== null) {
                    throw new Error("out-of-range varying index must return null");
                }
                "#,
            )
            .expect("transform feedback varying script should complete");
        responder
            .join()
            .expect("TF varying responder should exit cleanly");
    }
}

/// Compile-time test: does this `GLCmd` variant carry a heap
/// payload large enough that it could single-handedly blow the
/// collector's 4 MiB soft budget?
///
/// `false` for scalar variants (viewport, bind*, uniform scalars,
/// enable/disable, draw, scissor, ...) - the overwhelming majority
/// of ops by count in a real frame.  `true` for `BufferData` /
/// `TexImage2D` / `ShaderSource` / uniform array uploads, where a
/// single call can be megabytes.
///
/// Branching on this lets `queue_gl_fire_and_forget` skip both the
/// heavy `approx_deep_size_bytes` match AND the `maybe_auto_flush`
/// OpState re-borrow on the scalar fast path, without forcing every
/// call site to pick between `push_gl` / `push_gl_fast` manually.
/// With LTO the `matches!` compiles down to a handful of discriminant
/// comparisons (jump table), so the fast path remains cheap.
#[inline(always)]
fn gl_cmd_has_heap_payload(cmd: &GLCmd) -> bool {
    matches!(
        cmd,
        GLCmd::BufferData { .. }
            | GLCmd::BufferSubData { .. }
            | GLCmd::TexImage2D { .. }
            | GLCmd::TexSubImage2D { .. }
            | GLCmd::CompressedTexImage2D { .. }
            | GLCmd::CompressedTexSubImage2D { .. }
            | GLCmd::ShaderSource { .. }
            | GLCmd::GetUniformLocation { .. }
            | GLCmd::GetAttribLocation { .. }
            | GLCmd::BindAttribLocation { .. }
            | GLCmd::GetUniformBlockIndex { .. }
            | GLCmd::Uniform1iv { .. }
            | GLCmd::Uniform2iv { .. }
            | GLCmd::Uniform3iv { .. }
            | GLCmd::Uniform4iv { .. }
            | GLCmd::Uniform1fv { .. }
            | GLCmd::Uniform2fv { .. }
            | GLCmd::Uniform3fv { .. }
            | GLCmd::Uniform4fv { .. }
            | GLCmd::UniformMatrix2fv { .. }
            | GLCmd::UniformMatrix3fv { .. }
            | GLCmd::UniformMatrix4fv { .. }
            | GLCmd::InvalidateFramebuffer { .. }
            | GLCmd::DrawBuffers { .. }
            | GLCmd::TransformFeedbackVaryings { .. }
            | GLCmd::TexImage3D { .. }
            | GLCmd::TexSubImage3D { .. }
    )
}

#[inline]
pub(crate) fn queue_gl_fire_and_forget(state: &mut OpState, cmd: GLCmd) {
    let heap = gl_cmd_has_heap_payload(&cmd);
    let Some(collector) =
        state.try_borrow_mut::<crate::rendering::webgl::frame_collector::UnifiedFrameCollector>()
    else {
        error!("UnifiedFrameCollector missing in op state");
        return;
    };
    if heap {
        collector.push_gl(cmd);
        // Soft byte-budget backpressure: when a single push has
        // pushed the accumulated batch past the 4 MB threshold
        // (typically a `bufferData` / `texImage2D` with a fat
        // payload), cut the barrier here so we don't hold tens
        // of MB of heap on the JS thread while waiting for a
        // frame boundary.  Mirrors Chromium's
        // `CanvasResourceProvider::auto_flush` which uses its own
        // byte estimate to decide when to forcibly commit.
        maybe_auto_flush(state);
    } else {
        // Scalar fast path: accounted at `size_of::<GLCmd>()`,
        // no auto-flush check.  Scalar commands alone never blow
        // the soft budget - a bind/uniform storm of 100 000 calls
        // at ~128 B enum size tops out around 12 MiB, and any
        // frame holding that many GL ops is already pathological.
        collector.push_gl_fast(cmd);
    }
}

/// Inspect the frame collector; flush a non-presenting barrier if
/// the pending-bytes estimate has crossed
/// [`crate::rendering::webgl::frame_collector::AUTO_FLUSH_SOFT_BUDGET_BYTES`].
///
/// Kept separate so other push entry points (Canvas2D ops, future
/// side-channel payload paths) share the same trigger.
#[inline]
pub(crate) fn maybe_auto_flush(state: &mut OpState) {
    // Peek in a separate borrow scope so `flush_unified_barrier`
    // can re-acquire the collector mutably.
    let over_budget = state
        .try_borrow::<crate::rendering::webgl::frame_collector::UnifiedFrameCollector>()
        .map(|c| c.should_auto_flush())
        .unwrap_or(false);
    if over_budget {
        crate::rendering::webgl::frame_collector::flush_unified_barrier(state);
    }
}

#[inline]
fn send_gl_sync_with_flush<T>(
    state: &mut OpState,
    build: impl FnOnce(RenderCmdResp<T>) -> RenderCommand,
) -> Result<T, EngineError> {
    crate::rendering::webgl::frame_collector::flush_unified_barrier(state);
    let ctx = state.borrow::<CanvasOpState>();
    send_gl_with_resp_sync(ctx, build)
}

/// Result of trying to resolve RGBA bytes for a caller `image_id`.
///
/// Encodes the distinction between "the caller is referencing an id
/// we've never seen" and "we know the id but the bytes are missing"
/// so the miss-diagnostic log can tell the two failure modes apart.
enum RgbaLookup {
    Found {
        width: i32,
        height: i32,
        data: Arc<Vec<u8>>,
        /// Which code path served the bytes.  Used only for the
        /// `warn!` miss log — it tells us whether the pinned path
        /// (H-1) is doing its job on the next production drop.
        #[allow(dead_code)]
        source: RgbaSource,
    },
    UnknownAlias,
    AliasKnownButEvicted {
        cache_key: crate::rendering::image::cache::ImageCacheKey,
    },
}

#[derive(Debug)]
enum RgbaSource {
    /// Bytes came from the pin-protected io::global_cache.  This
    /// is the only path post-H-5; the variant is kept so future
    /// alternate sources (GPU-copy, direct-from-Skia) can be
    /// distinguished in diagnostic logs without a schema change.
    #[allow(dead_code)]
    IoCache,
}

#[inline]
fn resolve_cached_image_rgba(image_id: u32) -> RgbaLookup {
    // H-5: the io::global_cache is now the single source of truth
    // for decoded RGBA bytes, with `pin()` / `unpin()` keeping
    // actively referenced entries exempt from LRU eviction.  The
    // js-runtime IMAGE_CACHE just tells us whether we have an
    // alias for this caller `image_id` at all (and maps it to
    // the canonical cache key); the byte lookup then runs
    // against io::global_cache directly.
    //
    // The alias-known-but-evicted branch therefore only fires
    // when something outside the pin path has cleared the LRU
    // (e.g. `image_cache::global_cache().clear()` called
    // manually, or a pin-mismatch bug — both of which we want to
    // surface in the warn log rather than paper over silently).
    let key = {
        let c = IMAGE_CACHE.lock();
        c.cache_key_for_image_id(image_id)
    };
    let Some(key) = key else {
        return RgbaLookup::UnknownAlias;
    };

    let cached = {
        let mut cache = io::global_cache();
        cache.get(&crate::rendering::image::cache::to_io_cache_key(&key))
    };
    match cached {
        Some(entry) => {
            // Diag: trace confirms WebGL texImage2D actually
            // found bytes for `image_id`.  Used to verify the
            // H-5 pin path is keeping live aliases resident —
            // a spike of warn-level `miss (bytes evicted)` logs
            // would signal the pin is leaking.
            tracing::trace!(
                image_id,
                path = key.0.as_str(),
                gen = key.1,
                width = entry.image.width,
                height = entry.image.height,
                "resolve_cached_image_rgba hit"
            );
            RgbaLookup::Found {
                width: entry.image.width as i32,
                height: entry.image.height as i32,
                data: Arc::clone(&entry.image.rgba),
                source: RgbaSource::IoCache,
            }
        }
        None => RgbaLookup::AliasKnownButEvicted { cache_key: key },
    }
}

#[inline]
fn load_cached_image_rgba(image_id: u32) -> Option<(i32, i32, Arc<Vec<u8>>)> {
    match resolve_cached_image_rgba(image_id) {
        RgbaLookup::Found {
            width,
            height,
            data,
            ..
        } => Some((width, height, data)),
        _ => None,
    }
}

#[op2(fast)]
pub fn op_alloc_gl_resource_id(state: &mut OpState) -> u32 {
    let Some(alloc) = state.try_borrow_mut::<GlResourceIdAllocator>() else {
        error!("GlResourceIdAllocator missing in op state");
        return 0;
    };
    alloc.alloc()
}

#[op2(fast)]
pub fn op_gl_flush(state: &mut OpState) {
    crate::rendering::webgl::frame_collector::flush_unified_barrier(state);
}

#[op2(fast)]
pub fn op_viewport(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] x: i32,
    #[smi] y: i32,
    #[smi] width: u32,
    #[smi] height: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::Viewport {
            canvas_id,
            x,
            y,
            width,
            height,
        },
    );
}

#[op2(fast)]
pub fn op_clear_color(state: &mut OpState, #[smi] canvas_id: u32, r: f32, g: f32, b: f32, a: f32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::ClearColor {
            canvas_id,
            r,
            g,
            b,
            a,
        },
    );
}

#[op2(fast)]
pub fn op_clear(state: &mut OpState, #[smi] canvas_id: u32, #[smi] bit_field: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::Clear {
            canvas_id,
            bit_field,
        },
    );
}

#[op2(fast)]
pub fn op_create_program(state: &mut OpState, #[smi] canvas_id: u32, #[smi] client_id: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::CreateProgram {
            canvas_id,
            client_id,
        },
    );
}

#[op2(fast)]
pub fn op_use_program(state: &mut OpState, #[smi] canvas_id: u32, #[smi] program_id: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::UseProgram {
            canvas_id,
            program_id,
        },
    );
}

#[op2(fast)]
pub fn op_link_program(state: &mut OpState, #[smi] program_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::LinkProgram { program_id });
}

#[op2(fast)]
pub fn op_get_program_parameter(
    state: &mut OpState,
    #[smi] program_id: u32,
    #[smi] pname: u32,
) -> i32 {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetProgramParameter {
            program_id,
            pname,
            resp,
        })
    })
    .unwrap_or(0)
}

#[op2]
#[string]
pub fn op_get_program_info_log(state: &mut OpState, #[smi] program_id: u32) -> String {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetProgramInfoLog { program_id, resp })
    })
    .ok()
    .flatten()
    .unwrap_or_default()
}

#[op2(fast)]
pub fn op_delete_program(state: &mut OpState, #[smi] program_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DeleteProgram { program_id });
}

#[op2(fast)]
pub fn op_create_shader(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] client_id: u32,
    #[smi] ty: u32,
) {
    let shader_type = match ty {
        glow::VERTEX_SHADER => ShaderType::Vertex,
        glow::FRAGMENT_SHADER => ShaderType::Fragment,
        _ => {
            error!("unknown shader type: {}", ty);
            return;
        }
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::CreateShader {
            canvas_id,
            client_id,
            shader_type,
        },
    );
}

#[op2(fast)]
pub fn op_shader_source(state: &mut OpState, #[smi] shader_id: u32, #[string] source: String) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::ShaderSource {
            shader_id,
            source,
            resp: None,
        },
    );
}

#[op2(fast)]
pub fn op_compile_shader(state: &mut OpState, #[smi] shader_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::CompileShader { shader_id });
}

#[op2(fast)]
pub fn op_attach_shader(state: &mut OpState, #[smi] program_id: u32, #[smi] shader_id: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::AttachShader {
            program_id,
            shader_id,
            resp: None,
        },
    );
}

#[op2(fast)]
pub fn op_get_shader_parameter(
    state: &mut OpState,
    #[smi] shader_id: u32,
    #[smi] pname: u32,
) -> i32 {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetShaderParameter {
            shader_id,
            pname,
            resp,
        })
    })
    .unwrap_or(0)
}

#[op2]
#[string]
pub fn op_get_shader_info_log(state: &mut OpState, #[smi] shader_id: u32) -> String {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetShaderInfoLog { shader_id, resp })
    })
    .ok()
    .flatten()
    .unwrap_or_default()
}

#[op2(fast)]
pub fn op_delete_shader(state: &mut OpState, #[smi] shader_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DeleteShader { shader_id });
}

#[op2(fast)]
pub fn op_draw_arrays(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] mode: u32,
    #[smi] first: i32,
    #[smi] count: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::DrawArrays {
            canvas_id,
            mode,
            first,
            count,
        },
    );
}

#[op2(fast)]
pub fn op_draw_elements(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] mode: u32,
    #[smi] count: i32,
    #[smi] index_type: u32,
    #[smi] offset: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::DrawElements {
            canvas_id,
            mode,
            count,
            index_type,
            offset,
        },
    );
}

#[op2(fast)]
pub fn op_get_attrib_location(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] program_id: u32,
    #[string] name: String,
) -> i32 {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetAttribLocation {
            canvas_id,
            program_id,
            name,
            resp,
        })
    })
    .ok()
    .flatten()
    .map(|v| v as i32)
    .unwrap_or(-1)
}

#[op2(fast)]
pub fn op_bind_attrib_location(
    state: &mut OpState,
    #[smi] program_id: u32,
    #[smi] index: u32,
    #[string] name: String,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::BindAttribLocation {
            program_id,
            index,
            name,
        },
    );
}

#[op2]
#[string]
pub fn op_get_active_attrib(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] program_id: u32,
    #[smi] index: u32,
) -> String {
    let info = send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetActiveAttrib {
            canvas_id,
            program_id,
            index,
            resp,
        })
    })
    .ok()
    .flatten();

    if let Some((name, size, type_)) = info {
        let escaped_name = escape_for_json_string(&name);
        return format!(
            "{{\"name\":\"{}\",\"size\":{},\"type\":{}}}",
            escaped_name, size, type_
        );
    }

    String::new()
}

#[op2]
#[string]
pub fn op_get_active_uniform(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] program_id: u32,
    #[smi] index: u32,
) -> String {
    let info = send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetActiveUniform {
            canvas_id,
            program_id,
            index,
            resp,
        })
    })
    .ok()
    .flatten();

    if let Some((name, size, type_)) = info {
        let escaped_name = escape_for_json_string(&name);
        return format!(
            "{{\"name\":\"{}\",\"size\":{},\"type\":{}}}",
            escaped_name, size, type_
        );
    }

    String::new()
}

#[op2(fast)]
pub fn op_enable_vertex_attrib_array(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] index: u32,
) {
    queue_gl_fire_and_forget(state, GLCmd::EnableVertexAttribArray { canvas_id, index });
}

#[op2(fast)]
pub fn op_vertex_attrib_pointer(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] index: u32,
    #[smi] size: i32,
    #[smi] type_: u32,
    normalized: bool,
    #[smi] stride: i32,
    #[smi] offset: i32,
) {
    // Host-side validation: bad enum / out-of-range arguments push
    // a WebGL error into the per-context queue and the call is
    // NOT forwarded to the render thread.  Matches how
    // Firefox/Chromium reject invalid `vertexAttribPointer` args
    // before they reach the driver.
    if !crate::rendering::webgl::error_state::validate_vertex_attrib_pointer(
        state, canvas_id, size, type_, stride, offset,
    ) {
        return;
    }
    queue_gl_fire_and_forget(
        state,
        GLCmd::VertexAttribPointer {
            canvas_id,
            index,
            size,
            type_,
            normalized,
            stride,
            offset,
        },
    );
}

#[op2(fast)]
pub fn op_create_buffer(state: &mut OpState, #[smi] canvas_id: u32, #[smi] client_id: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::CreateBuffer {
            canvas_id,
            client_id,
        },
    );
}

#[op2(fast)]
pub fn op_bind_buffer(state: &mut OpState, #[smi] canvas_id: u32, #[smi] target: u32, buffer: i32) {
    // Host-side target-enum validation — the render thread's
    // `BindBuffer` dispatcher silently ignores unknown targets,
    // which hides real bugs; surface them via `getError()`
    // instead.  ARRAY_BUFFER / ELEMENT_ARRAY_BUFFER always legal;
    // WebGL 2 targets (UNIFORM_BUFFER etc.) are legal too but
    // only usefully bound on a WebGL 2 context — the op doesn't
    // know its own context version here, so we allow all legal
    // GL ES 3.0 targets and rely on the render thread to reject
    // the ones that don't apply.
    if !crate::rendering::webgl::error_state::validate_bind_buffer_target(state, canvas_id, target)
    {
        return;
    }
    let buffer = if buffer < 0 {
        None
    } else {
        Some(buffer as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::BindBuffer {
            canvas_id,
            target,
            buffer,
        },
    );
}

#[op2]
pub fn op_buffer_data(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] size: i32,
    // #[buffer(copy)] gives us an owned Vec<u8> directly, avoiding an
    // intermediate JsBuffer wrapper + separate .to_vec() heap allocation.
    // The copy itself is unavoidable: V8 owns the ArrayBuffer backing store
    // and we must send owned data to the render thread.
    #[buffer(copy)] data: Option<Vec<u8>>,
    #[smi] usage: u32,
) {
    if data.is_none() && size <= 0 {
        error!("op_buffer_data: size must > 0 when data is None");
        return;
    }

    queue_gl_fire_and_forget(
        state,
        GLCmd::BufferData {
            canvas_id,
            target,
            size,
            data,
            usage,
        },
    );
}

#[op2(fast)]
pub fn op_get_uniform_location(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] program_id: u32,
    #[string] name: String,
) -> i32 {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetUniformLocation {
            canvas_id,
            program_id,
            name,
            resp,
        })
    })
    .ok()
    .flatten()
    .map(|v| v as i32)
    .unwrap_or(-1)
}

#[op2(fast)]
pub fn op_uniform3f(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    x: f32,
    y: f32,
    z: f32,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform3f {
            canvas_id,
            location,
            x,
            y,
            z,
        },
    );
}

#[op2(fast)]
pub fn op_uniform_matrix_3fv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    transpose: bool,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<f32> = cast_vec(value);

    queue_gl_fire_and_forget(
        state,
        GLCmd::UniformMatrix3fv {
            canvas_id,
            location,
            transpose,
            value,
        },
    );
}

// ---------------------------------------------------------------------------
// Phase 1A: GL State
// ---------------------------------------------------------------------------

#[op2(fast)]
pub fn op_enable(state: &mut OpState, #[smi] canvas_id: u32, #[smi] cap: u32) {
    queue_gl_fire_and_forget(state, GLCmd::Enable { canvas_id, cap });
}

#[op2(fast)]
pub fn op_disable(state: &mut OpState, #[smi] canvas_id: u32, #[smi] cap: u32) {
    queue_gl_fire_and_forget(state, GLCmd::Disable { canvas_id, cap });
}

#[op2(fast)]
pub fn op_is_enabled(state: &mut OpState, #[smi] canvas_id: u32, #[smi] cap: u32) -> u32 {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::IsEnabled {
            canvas_id,
            cap,
            resp,
        })
    })
    .map(|v| if v { 1 } else { 0 })
    .unwrap_or(0)
}

/// PERF: Architectural limitation -- this is a synchronous cross-thread call.
/// `op_get_parameter` flushes the pending GL command batch, sends a
/// `GetParameter` request to the render thread, and blocks the JS thread
/// until the render thread processes it and responds.  This causes a full
/// pipeline stall: JS cannot execute while waiting, and the render thread
/// must drain its queue to reach this request.
///
/// Frequent calls (e.g. inside a draw loop) will significantly degrade
/// frame rate.  Games should cache parameter values on the JS side
/// when possible.
///
/// Note: `gl.getError()` is currently stubbed to always return 0 on the JS
/// side (`02_webgl_context.js`), so it does not hit this path.  If a real
/// implementation is ever needed, consider maintaining a last-error cache
/// on the render thread updated by each GL call, and reading it via a
/// lock-free atomic instead of a sync round-trip.
#[op2]
#[string]
pub fn op_get_parameter(state: &mut OpState, #[smi] canvas_id: u32, #[smi] pname: u32) -> String {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetParameter {
            canvas_id,
            pname,
            resp,
        })
    })
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Phase 1B: Textures
// ---------------------------------------------------------------------------

#[op2(fast)]
pub fn op_create_texture(state: &mut OpState, #[smi] canvas_id: u32, #[smi] client_id: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::CreateTexture {
            canvas_id,
            client_id,
        },
    );
}

#[op2(fast)]
pub fn op_delete_texture(state: &mut OpState, #[smi] texture_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DeleteTexture { texture_id });
}

#[op2(fast)]
pub fn op_bind_texture(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    texture: i32,
) {
    let texture = if texture < 0 {
        None
    } else {
        Some(texture as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::BindTexture {
            canvas_id,
            target,
            texture,
        },
    );
}

#[op2(fast)]
pub fn op_active_texture(state: &mut OpState, #[smi] canvas_id: u32, #[smi] unit: u32) {
    queue_gl_fire_and_forget(state, GLCmd::ActiveTexture { canvas_id, unit });
}

#[op2]
pub fn op_tex_image_2d(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] internalformat: i32,
    #[smi] width: i32,
    #[smi] height: i32,
    #[smi] border: i32,
    #[smi] format: u32,
    #[smi] type_: u32,
    // #[buffer(copy)] -> owned Vec<u8>; avoids intermediate JsBuffer + .to_vec().
    #[buffer(copy)] data: Option<Vec<u8>>,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::TexImage2D {
            canvas_id,
            target,
            level,
            internalformat,
            width,
            height,
            border,
            format,
            type_,
            data: data.map(Arc::new),
        },
    );
}

#[op2(fast)]
pub fn op_tex_image_2d_from_image(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] internalformat: i32,
    #[smi] format: u32,
    #[smi] type_: u32,
    #[smi] image_id: u32,
) {
    // Fast path: the image has already been uploaded to a GL texture
    // by `op_load_image` (CanvasCmd::LoadImage path) and is sitting
    // in the render thread's `ImageStore`.  We hand the destination
    // upload off as a GPU-side copy from that existing texture, so
    // the WebGL texImage2D round-trip never re-reads the CPU-side
    // RGBA bytes.  Mirrors what Chrome does for HTMLImageElement →
    // gl.texImage2D after the bitmap has been promoted to a GPU
    // texture.
    let shared = {
        let c = crate::rendering::image::cache::IMAGE_CACHE.lock();
        c.shared_for_image_id(image_id)
    };
    if let Some((source_shared_id, (w, h))) = shared {
        queue_gl_fire_and_forget(
            state,
            GLCmd::TexImage2DFromShared {
                canvas_id,
                target,
                level,
                internalformat,
                format,
                type_,
                source_shared_id,
                src_width: w as i32,
                src_height: h as i32,
            },
        );
        return;
    }

    // Slow path: the image's GL texture is not (yet) live in the
    // store but the decoded RGBA bytes are still in the io cache —
    // re-upload from CPU bytes.  Also covers the diagnostic miss
    // classes (unknown alias / evicted bytes).
    let (width, height, data) = match resolve_cached_image_rgba(image_id) {
        RgbaLookup::Found {
            width,
            height,
            data,
            ..
        } => (width, height, data),
        RgbaLookup::UnknownAlias => {
            warn!(
                "op_tex_image_2d_from_image miss (unknown alias): image_id={}",
                image_id
            );
            return;
        }
        RgbaLookup::AliasKnownButEvicted { cache_key } => {
            warn!(
                "op_tex_image_2d_from_image miss (bytes evicted): image_id={}, src={}, gen={}",
                image_id, cache_key.0, cache_key.1
            );
            return;
        }
    };

    queue_gl_fire_and_forget(
        state,
        GLCmd::TexImage2D {
            canvas_id,
            target,
            level,
            internalformat,
            width,
            height,
            border: 0,
            format,
            type_,
            data: Some(data),
        },
    );
}

/// `texImage2D` from a Canvas2D snapshot allocated by
/// `op_get_image_data_snapshot`.  Routes a single `GLCmd` into the
/// frame collector so the upload lands inside the same FramePacket
/// as the surrounding WebGL draw — no inserted Materialize barrier,
/// no sync flush, no CPU readback.  Mirrors
/// [`op_tex_image_2d_from_image`].
#[op2(fast)]
pub fn op_tex_image_2d_from_snapshot(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] internalformat: i32,
    #[smi] format: u32,
    #[smi] type_: u32,
    #[smi] snapshot_id: u32,
) {
    if snapshot_id == 0 {
        // JS-side fallback already happened (`getImageData` returned
        // a real CPU buffer instead of a snapshot wrapper); this op
        // shouldn't be invoked.  Drop silently to keep the call site
        // total; tracing-warn would just spam logs on misuse.
        return;
    }
    queue_gl_fire_and_forget(
        state,
        GLCmd::TexImage2DFromSnapshot {
            canvas_id,
            target,
            level,
            internalformat,
            format,
            type_,
            snapshot_id,
        },
    );
}

/// Direct GPU->GPU `texImage2D` from an HTMLCanvasElement -- bypasses
/// the getImageData->snapshot->force-readback chain that cocos's
/// `gl.texImage2D(target, ..., canvasElement)` pattern was triggering
/// (~50ms V8 stall per call on the emulator, ~20 calls per popup).
/// Fire-and-forget: render thread does FBO blit + glCopyTexImage2D in
/// one shot.
#[op2(fast)]
pub fn op_tex_image_2d_from_canvas2d(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] internalformat: i32,
    #[smi] canvas_2d_id: u32,
    #[smi] x: i32,
    #[smi] y: i32,
    #[smi] width: u32,
    #[smi] height: u32,
) {
    if width == 0 || height == 0 {
        return;
    }
    queue_gl_fire_and_forget(
        state,
        GLCmd::TexImage2DFromCanvas2D {
            canvas_id,
            target,
            level,
            internalformat,
            canvas_2d_id,
            x,
            y,
            width,
            height,
        },
    );
}

#[op2(fast)]
pub fn op_tex_sub_image_2d_from_canvas2d(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] xoffset: i32,
    #[smi] yoffset: i32,
    #[smi] canvas_2d_id: u32,
    #[smi] x: i32,
    #[smi] y: i32,
    #[smi] width: u32,
    #[smi] height: u32,
) {
    if width == 0 || height == 0 {
        return;
    }
    queue_gl_fire_and_forget(
        state,
        GLCmd::TexSubImage2DFromCanvas2D {
            canvas_id,
            target,
            level,
            xoffset,
            yoffset,
            canvas_2d_id,
            x,
            y,
            width,
            height,
        },
    );
}

/// `texSubImage2D` from a Canvas2D snapshot -- sibling of
/// `op_tex_image_2d_from_snapshot` for cocos-style text atlases that
/// pre-allocate an atlas texture and stream glyph cells in via
/// `texSubImage2D`.  Without this op, the JS path falls through to
/// `op_tex_sub_image_2d` and uploads the zero-filled placeholder
/// `Uint8ClampedArray` carried by the synthetic ImageData -- visible
/// as missing glyphs in the atlas.
#[op2(fast)]
pub fn op_tex_sub_image_2d_from_snapshot(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] xoffset: i32,
    #[smi] yoffset: i32,
    #[smi] format: u32,
    #[smi] type_: u32,
    #[smi] snapshot_id: u32,
) {
    if snapshot_id == 0 {
        return;
    }
    queue_gl_fire_and_forget(
        state,
        GLCmd::TexSubImage2DFromSnapshot {
            canvas_id,
            target,
            level,
            xoffset,
            yoffset,
            format,
            type_,
            snapshot_id,
        },
    );
}

#[op2(fast)]
pub fn op_tex_sub_image_2d(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] xoffset: i32,
    #[smi] yoffset: i32,
    #[smi] width: i32,
    #[smi] height: i32,
    #[smi] format: u32,
    #[smi] type_: u32,
    // #[buffer(copy)] -> owned Vec<u8>; avoids intermediate JsBuffer + .to_vec().
    #[buffer(copy)] data: Vec<u8>,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::TexSubImage2D {
            canvas_id,
            target,
            level,
            xoffset,
            yoffset,
            width,
            height,
            format,
            type_,
            data: Arc::new(data),
        },
    );
}

#[op2(fast)]
pub fn op_tex_sub_image_2d_from_image(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] xoffset: i32,
    #[smi] yoffset: i32,
    #[smi] format: u32,
    #[smi] type_: u32,
    #[smi] image_id: u32,
) {
    let (width, height, data) = match resolve_cached_image_rgba(image_id) {
        RgbaLookup::Found {
            width,
            height,
            data,
            ..
        } => (width, height, data),
        RgbaLookup::UnknownAlias => {
            warn!(
                "op_tex_sub_image_2d_from_image miss (unknown alias): image_id={}",
                image_id
            );
            return;
        }
        RgbaLookup::AliasKnownButEvicted { cache_key } => {
            warn!(
                "op_tex_sub_image_2d_from_image miss (bytes evicted): image_id={}, src={}, gen={}",
                image_id, cache_key.0, cache_key.1
            );
            return;
        }
    };

    queue_gl_fire_and_forget(
        state,
        GLCmd::TexSubImage2D {
            canvas_id,
            target,
            level,
            xoffset,
            yoffset,
            width,
            height,
            format,
            type_,
            data,
        },
    );
}

#[op2(fast)]
pub fn op_tex_parameteri(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] pname: u32,
    #[smi] param: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::TexParameteri {
            canvas_id,
            target,
            pname,
            param,
        },
    );
}

#[op2(fast)]
pub fn op_tex_parameterf(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] pname: u32,
    param: f32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::TexParameterf {
            canvas_id,
            target,
            pname,
            param,
        },
    );
}

#[op2(fast)]
pub fn op_generate_mipmap(state: &mut OpState, #[smi] canvas_id: u32, #[smi] target: u32) {
    queue_gl_fire_and_forget(state, GLCmd::GenerateMipmap { canvas_id, target });
}

#[op2(fast)]
pub fn op_pixel_storei(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] pname: u32,
    #[smi] param: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::PixelStorei {
            canvas_id,
            pname,
            param,
        },
    );
}

#[op2(fast)]
pub fn op_compressed_tex_image_2d(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] internalformat: u32,
    #[smi] width: i32,
    #[smi] height: i32,
    #[smi] border: i32,
    // #[buffer(copy)] -> owned Vec<u8>; avoids intermediate JsBuffer + .to_vec().
    #[buffer(copy)] data: Vec<u8>,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::CompressedTexImage2D {
            canvas_id,
            target,
            level,
            internalformat,
            width,
            height,
            border,
            data,
        },
    );
}

#[op2(fast)]
pub fn op_compressed_tex_sub_image_2d(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] xoffset: i32,
    #[smi] yoffset: i32,
    #[smi] width: i32,
    #[smi] height: i32,
    #[smi] format: u32,
    // #[buffer(copy)] -> owned Vec<u8>; avoids intermediate JsBuffer + .to_vec().
    #[buffer(copy)] data: Vec<u8>,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::CompressedTexSubImage2D {
            canvas_id,
            target,
            level,
            xoffset,
            yoffset,
            width,
            height,
            format,
            data,
        },
    );
}

// ---------------------------------------------------------------------------
// Phase 1C: Buffer & Vertex Extensions
// ---------------------------------------------------------------------------

#[op2(fast)]
pub fn op_buffer_sub_data(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] offset: i32,
    // #[buffer(copy)] -> owned Vec<u8>; avoids intermediate JsBuffer + .to_vec().
    #[buffer(copy)] data: Vec<u8>,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::BufferSubData {
            canvas_id,
            target,
            offset,
            data,
        },
    );
}

#[op2(fast)]
pub fn op_disable_vertex_attrib_array(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] index: u32,
) {
    queue_gl_fire_and_forget(state, GLCmd::DisableVertexAttribArray { canvas_id, index });
}

#[op2(fast)]
pub fn op_clear_depth(state: &mut OpState, #[smi] canvas_id: u32, depth: f32) {
    queue_gl_fire_and_forget(state, GLCmd::ClearDepth { canvas_id, depth });
}

#[op2(fast)]
pub fn op_clear_stencil(state: &mut OpState, #[smi] canvas_id: u32, #[smi] s: i32) {
    queue_gl_fire_and_forget(state, GLCmd::ClearStencil { canvas_id, s });
}

// ---------------------------------------------------------------------------
// Phase 2A: Blend / Depth / Stencil / Cull State
// ---------------------------------------------------------------------------

#[op2(fast)]
pub fn op_blend_func(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] sfactor: u32,
    #[smi] dfactor: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::BlendFunc {
            canvas_id,
            sfactor,
            dfactor,
        },
    );
}

#[op2(fast)]
pub fn op_blend_func_separate(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] src_rgb: u32,
    #[smi] dst_rgb: u32,
    #[smi] src_alpha: u32,
    #[smi] dst_alpha: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::BlendFuncSeparate {
            canvas_id,
            src_rgb,
            dst_rgb,
            src_alpha,
            dst_alpha,
        },
    );
}

#[op2(fast)]
pub fn op_blend_equation(state: &mut OpState, #[smi] canvas_id: u32, #[smi] mode: u32) {
    queue_gl_fire_and_forget(state, GLCmd::BlendEquation { canvas_id, mode });
}

#[op2(fast)]
pub fn op_blend_equation_separate(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] mode_rgb: u32,
    #[smi] mode_alpha: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::BlendEquationSeparate {
            canvas_id,
            mode_rgb,
            mode_alpha,
        },
    );
}

#[op2(fast)]
pub fn op_blend_color(state: &mut OpState, #[smi] canvas_id: u32, r: f32, g: f32, b: f32, a: f32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::BlendColor {
            canvas_id,
            r,
            g,
            b,
            a,
        },
    );
}

#[op2(fast)]
pub fn op_depth_func(state: &mut OpState, #[smi] canvas_id: u32, #[smi] func: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DepthFunc { canvas_id, func });
}

#[op2(fast)]
pub fn op_depth_mask(state: &mut OpState, #[smi] canvas_id: u32, flag: bool) {
    queue_gl_fire_and_forget(state, GLCmd::DepthMask { canvas_id, flag });
}

#[op2(fast)]
pub fn op_depth_range(state: &mut OpState, #[smi] canvas_id: u32, near: f32, far: f32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::DepthRange {
            canvas_id,
            near,
            far,
        },
    );
}

#[op2(fast)]
pub fn op_stencil_func(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] func: u32,
    #[smi] ref_: i32,
    #[smi] mask: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::StencilFunc {
            canvas_id,
            func,
            ref_,
            mask,
        },
    );
}

#[op2(fast)]
pub fn op_stencil_func_separate(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] face: u32,
    #[smi] func: u32,
    #[smi] ref_: i32,
    #[smi] mask: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::StencilFuncSeparate {
            canvas_id,
            face,
            func,
            ref_,
            mask,
        },
    );
}

#[op2(fast)]
pub fn op_stencil_op(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] fail: u32,
    #[smi] zfail: u32,
    #[smi] zpass: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::StencilOp {
            canvas_id,
            fail,
            zfail,
            zpass,
        },
    );
}

#[op2(fast)]
pub fn op_stencil_op_separate(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] face: u32,
    #[smi] fail: u32,
    #[smi] zfail: u32,
    #[smi] zpass: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::StencilOpSeparate {
            canvas_id,
            face,
            fail,
            zfail,
            zpass,
        },
    );
}

#[op2(fast)]
pub fn op_stencil_mask(state: &mut OpState, #[smi] canvas_id: u32, #[smi] mask: u32) {
    queue_gl_fire_and_forget(state, GLCmd::StencilMask { canvas_id, mask });
}

#[op2(fast)]
pub fn op_stencil_mask_separate(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] face: u32,
    #[smi] mask: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::StencilMaskSeparate {
            canvas_id,
            face,
            mask,
        },
    );
}

#[op2(fast)]
pub fn op_cull_face(state: &mut OpState, #[smi] canvas_id: u32, #[smi] mode: u32) {
    queue_gl_fire_and_forget(state, GLCmd::CullFace { canvas_id, mode });
}

#[op2(fast)]
pub fn op_front_face(state: &mut OpState, #[smi] canvas_id: u32, #[smi] mode: u32) {
    queue_gl_fire_and_forget(state, GLCmd::FrontFace { canvas_id, mode });
}

#[op2(fast)]
pub fn op_color_mask(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    r: bool,
    g: bool,
    b: bool,
    a: bool,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::ColorMask {
            canvas_id,
            r,
            g,
            b,
            a,
        },
    );
}

#[op2(fast)]
pub fn op_scissor(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] x: i32,
    #[smi] y: i32,
    #[smi] width: i32,
    #[smi] height: i32,
) {
    // WebGL spec: negative width/height → INVALID_VALUE.
    if !crate::rendering::webgl::error_state::validate_viewport_like(
        state, canvas_id, width, height,
    ) {
        return;
    }
    queue_gl_fire_and_forget(
        state,
        GLCmd::Scissor {
            canvas_id,
            x,
            y,
            width,
            height,
        },
    );
}

#[op2(fast)]
pub fn op_line_width(state: &mut OpState, #[smi] canvas_id: u32, width: f32) {
    queue_gl_fire_and_forget(state, GLCmd::LineWidth { canvas_id, width });
}

#[op2(fast)]
pub fn op_polygon_offset(state: &mut OpState, #[smi] canvas_id: u32, factor: f32, units: f32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::PolygonOffset {
            canvas_id,
            factor,
            units,
        },
    );
}

// ---------------------------------------------------------------------------
// Phase 2B: Uniform Variants
// ---------------------------------------------------------------------------

#[op2(fast)]
pub fn op_uniform1i(state: &mut OpState, #[smi] canvas_id: u32, location: i32, #[smi] x: i32) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform1i {
            canvas_id,
            location,
            x,
        },
    );
}

#[op2(fast)]
pub fn op_uniform1f(state: &mut OpState, #[smi] canvas_id: u32, location: i32, x: f32) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform1f {
            canvas_id,
            location,
            x,
        },
    );
}

#[op2(fast)]
pub fn op_uniform2f(state: &mut OpState, #[smi] canvas_id: u32, location: i32, x: f32, y: f32) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform2f {
            canvas_id,
            location,
            x,
            y,
        },
    );
}

#[op2(fast)]
pub fn op_uniform4f(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    x: f32,
    y: f32,
    z: f32,
    w: f32,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform4f {
            canvas_id,
            location,
            x,
            y,
            z,
            w,
        },
    );
}

#[op2(fast)]
pub fn op_uniform1iv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<i32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform1iv {
            canvas_id,
            location,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform1fv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<f32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform1fv {
            canvas_id,
            location,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform2iv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<i32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform2iv {
            canvas_id,
            location,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform2fv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<f32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform2fv {
            canvas_id,
            location,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform3iv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<i32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform3iv {
            canvas_id,
            location,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform3fv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<f32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform3fv {
            canvas_id,
            location,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform4iv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<i32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform4iv {
            canvas_id,
            location,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform4fv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<f32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::Uniform4fv {
            canvas_id,
            location,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform_matrix_2fv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    transpose: bool,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<f32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::UniformMatrix2fv {
            canvas_id,
            location,
            transpose,
            value,
        },
    );
}

#[op2(fast)]
pub fn op_uniform_matrix_4fv(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    location: i32,
    transpose: bool,
    #[buffer(copy)] value: Vec<u32>,
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value: Vec<f32> = cast_vec(value);
    queue_gl_fire_and_forget(
        state,
        GLCmd::UniformMatrix4fv {
            canvas_id,
            location,
            transpose,
            value,
        },
    );
}

// ---------------------------------------------------------------------------
// Phase 3A: Framebuffer / Renderbuffer
// ---------------------------------------------------------------------------

#[op2(fast)]
pub fn op_create_framebuffer(state: &mut OpState, #[smi] canvas_id: u32, #[smi] client_id: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::CreateFramebuffer {
            canvas_id,
            client_id,
        },
    );
}

#[op2(fast)]
pub fn op_delete_framebuffer(state: &mut OpState, #[smi] framebuffer_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DeleteFramebuffer { framebuffer_id });
}

#[op2(fast)]
pub fn op_bind_framebuffer(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    framebuffer: i32,
) {
    let framebuffer = if framebuffer < 0 {
        None
    } else {
        Some(framebuffer as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::BindFramebuffer {
            canvas_id,
            target,
            framebuffer,
        },
    );
}

#[op2(fast)]
pub fn op_framebuffer_texture_2d(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] attachment: u32,
    #[smi] textarget: u32,
    texture: i32,
    #[smi] level: i32,
) {
    let texture = if texture < 0 {
        None
    } else {
        Some(texture as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::FramebufferTexture2D {
            canvas_id,
            target,
            attachment,
            textarget,
            texture,
            level,
        },
    );
}

#[op2(fast)]
pub fn op_framebuffer_renderbuffer(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] attachment: u32,
    #[smi] renderbuffertarget: u32,
    renderbuffer: i32,
) {
    let renderbuffer = if renderbuffer < 0 {
        None
    } else {
        Some(renderbuffer as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::FramebufferRenderbuffer {
            canvas_id,
            target,
            attachment,
            renderbuffertarget,
            renderbuffer,
        },
    );
}

#[op2(fast)]
pub fn op_check_framebuffer_status(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
) -> u32 {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::CheckFramebufferStatus {
            canvas_id,
            target,
            resp,
        })
    })
    .unwrap_or(0)
}

#[op2(fast)]
pub fn op_create_renderbuffer(state: &mut OpState, #[smi] canvas_id: u32, #[smi] client_id: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::CreateRenderbuffer {
            canvas_id,
            client_id,
        },
    );
}

#[op2(fast)]
pub fn op_delete_renderbuffer(state: &mut OpState, #[smi] renderbuffer_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DeleteRenderbuffer { renderbuffer_id });
}

#[op2(fast)]
pub fn op_bind_renderbuffer(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    renderbuffer: i32,
) {
    let renderbuffer = if renderbuffer < 0 {
        None
    } else {
        Some(renderbuffer as u32)
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::BindRenderbuffer {
            canvas_id,
            target,
            renderbuffer,
        },
    );
}

#[op2(fast)]
pub fn op_renderbuffer_storage(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] internalformat: u32,
    #[smi] width: i32,
    #[smi] height: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::RenderbufferStorage {
            canvas_id,
            target,
            internalformat,
            width,
            height,
        },
    );
}

// ---------------------------------------------------------------------------
// Phase 3B: Misc
// ---------------------------------------------------------------------------

#[op2]
#[buffer]
pub fn op_read_pixels(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] x: i32,
    #[smi] y: i32,
    #[smi] width: i32,
    #[smi] height: i32,
    #[smi] format: u32,
    #[smi] type_: u32,
) -> Vec<u8> {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::ReadPixels {
            canvas_id,
            x,
            y,
            width,
            height,
            format,
            type_,
            resp,
        })
    })
    .unwrap_or_default()
}

#[op2(fast)]
pub fn op_hint(state: &mut OpState, #[smi] canvas_id: u32, #[smi] target: u32, #[smi] mode: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::Hint {
            canvas_id,
            target,
            mode,
        },
    );
}

// ---------------------------------------------------------------------------
// WebGL 2.0 / GLES 3.0 additions
//
// Each op mirrors a single entry in GLCmd (see shared/protocol/render_cmd.rs).
// Fire-and-forget ops route through `queue_gl_fire_and_forget` so the
// UnifiedFrameCollector can batch them into the frame packet.  Sync ops
// (getUniformBlockIndex, clientWaitSync) call `send_gl_sync_with_flush`
// so any pending batch is materialised before the reply is waited on.
// ---------------------------------------------------------------------------

#[op2(fast)]
pub fn op_create_vertex_array(state: &mut OpState, #[smi] canvas_id: u32, #[smi] client_id: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::CreateVertexArray {
            canvas_id,
            client_id,
        },
    );
}

#[op2(fast)]
pub fn op_delete_vertex_array(state: &mut OpState, #[smi] vao: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DeleteVertexArray { vao });
}

#[op2(fast)]
pub fn op_bind_vertex_array(state: &mut OpState, #[smi] canvas_id: u32, #[smi] vao: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::BindVertexArray {
            canvas_id,
            vao: if vao == 0 { None } else { Some(vao) },
        },
    );
}

#[op2(fast)]
pub fn op_vertex_attrib_divisor(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] index: u32,
    #[smi] divisor: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::VertexAttribDivisor {
            canvas_id,
            index,
            divisor,
        },
    );
}

#[op2(fast)]
pub fn op_draw_arrays_instanced(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] mode: u32,
    #[smi] first: i32,
    #[smi] count: i32,
    #[smi] instance_count: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::DrawArraysInstanced {
            canvas_id,
            mode,
            first,
            count,
            instance_count,
        },
    );
}

#[op2(fast)]
pub fn op_draw_elements_instanced(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] mode: u32,
    #[smi] count: i32,
    #[smi] index_type: u32,
    #[smi] offset: i32,
    #[smi] instance_count: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::DrawElementsInstanced {
            canvas_id,
            mode,
            count,
            index_type,
            offset,
            instance_count,
        },
    );
}

#[op2(fast)]
#[smi]
pub fn op_get_uniform_block_index(
    state: &mut OpState,
    #[smi] program_id: u32,
    #[string] name: String,
) -> u32 {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetUniformBlockIndex {
            program_id,
            name,
            resp,
        })
    })
    .unwrap_or(u32::MAX)
}

#[op2(fast)]
pub fn op_uniform_block_binding(
    state: &mut OpState,
    #[smi] program_id: u32,
    #[smi] uniform_block_index: u32,
    #[smi] uniform_block_binding: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::UniformBlockBinding {
            program_id,
            uniform_block_index,
            uniform_block_binding,
        },
    );
}

#[op2(fast)]
pub fn op_bind_buffer_base(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] index: u32,
    #[smi] buffer: u32,
) {
    bind_buffer_base_impl(state, canvas_id, target, index, buffer);
}

fn bind_buffer_base_impl(
    state: &mut OpState,
    canvas_id: u32,
    target: u32,
    index: u32,
    buffer: u32,
) {
    let buffer = if buffer == 0 { None } else { Some(buffer) };
    if !crate::rendering::webgl::error_state::validate_bind_buffer_base(
        state, canvas_id, target, index, buffer,
    ) {
        return;
    }
    queue_gl_fire_and_forget(
        state,
        GLCmd::BindBufferBase {
            canvas_id,
            target,
            index,
            buffer,
        },
    );
}

#[op2(fast)]
pub fn op_bind_buffer_range(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] index: u32,
    #[smi] buffer: u32,
    #[smi] offset: i32,
    #[smi] size: i32,
) {
    bind_buffer_range_impl(state, canvas_id, target, index, buffer, offset, size);
}

fn bind_buffer_range_impl(
    state: &mut OpState,
    canvas_id: u32,
    target: u32,
    index: u32,
    buffer: u32,
    offset: i32,
    size: i32,
) {
    let buffer = if buffer == 0 { None } else { Some(buffer) };
    if !crate::rendering::webgl::error_state::validate_bind_buffer_range(
        state, canvas_id, target, index, buffer, offset, size,
    ) {
        return;
    }
    queue_gl_fire_and_forget(
        state,
        GLCmd::BindBufferRange {
            canvas_id,
            target,
            index,
            buffer,
            offset,
            size,
        },
    );
}

#[op2(fast)]
pub fn op_tex_storage_2d(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] levels: i32,
    #[smi] internal_format: u32,
    #[smi] width: i32,
    #[smi] height: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::TexStorage2D {
            canvas_id,
            target,
            levels,
            internal_format,
            width,
            height,
        },
    );
}

#[op2(fast)]
#[allow(clippy::too_many_arguments)]
pub fn op_blit_framebuffer(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] src_x0: i32,
    #[smi] src_y0: i32,
    #[smi] src_x1: i32,
    #[smi] src_y1: i32,
    #[smi] dst_x0: i32,
    #[smi] dst_y0: i32,
    #[smi] dst_x1: i32,
    #[smi] dst_y1: i32,
    #[smi] mask: u32,
    #[smi] filter: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::BlitFramebuffer {
            canvas_id,
            src_x0,
            src_y0,
            src_x1,
            src_y1,
            dst_x0,
            dst_y0,
            dst_x1,
            dst_y1,
            mask,
            filter,
        },
    );
}

#[op2(fast)]
pub fn op_invalidate_framebuffer(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    // JS passes a `Uint32Array` directly.  `#[buffer(copy)]` copies the
    // element view, yielding an owned Vec without bytemuck gymnastics.
    #[buffer(copy)] attachments: Vec<u32>,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::InvalidateFramebuffer {
            canvas_id,
            target,
            attachments,
        },
    );
}

#[op2(fast)]
pub fn op_renderbuffer_storage_multisample(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] samples: i32,
    #[smi] internal_format: u32,
    #[smi] width: i32,
    #[smi] height: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::RenderbufferStorageMultisample {
            canvas_id,
            target,
            samples,
            internal_format,
            width,
            height,
        },
    );
}

#[op2(fast)]
pub fn op_create_sampler(state: &mut OpState, #[smi] canvas_id: u32, #[smi] client_id: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::CreateSampler {
            canvas_id,
            client_id,
        },
    );
}

#[op2(fast)]
pub fn op_delete_sampler(state: &mut OpState, #[smi] sampler: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DeleteSampler { sampler });
}

#[op2(fast)]
pub fn op_bind_sampler(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] unit: u32,
    #[smi] sampler: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::BindSampler {
            canvas_id,
            unit,
            sampler: if sampler == 0 { None } else { Some(sampler) },
        },
    );
}

#[op2(fast)]
pub fn op_sampler_parameteri(
    state: &mut OpState,
    #[smi] sampler: u32,
    #[smi] pname: u32,
    #[smi] param: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::SamplerParameteri {
            sampler,
            pname,
            param,
        },
    );
}

#[op2(fast)]
pub fn op_sampler_parameterf(
    state: &mut OpState,
    #[smi] sampler: u32,
    #[smi] pname: u32,
    param: f32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::SamplerParameterf {
            sampler,
            pname,
            param,
        },
    );
}

#[op2(fast)]
pub fn op_fence_sync(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] client_id: u32,
    #[smi] condition: u32,
    #[smi] flags: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::FenceSync {
            canvas_id,
            client_id,
            condition,
            flags,
        },
    );
}

#[op2(fast)]
pub fn op_delete_sync(state: &mut OpState, #[smi] sync: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DeleteSync { sync });
}

#[op2(fast)]
#[smi]
pub fn op_client_wait_sync(
    state: &mut OpState,
    #[smi] sync: u32,
    #[smi] flags: u32,
    timeout_ns: f64,
) -> u32 {
    let timeout_ns = if timeout_ns.is_finite() && timeout_ns >= 0.0 {
        timeout_ns as u64
    } else {
        0
    };
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::ClientWaitSync {
            sync,
            flags,
            timeout_ns,
            resp,
        })
    })
    // WAIT_FAILED = 0x911D per GLES 3.0 spec.
    .unwrap_or(0x911D)
}

#[op2(fast)]
pub fn op_draw_buffers(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[buffer(copy)] buffers: Vec<u32>,
) {
    queue_gl_fire_and_forget(state, GLCmd::DrawBuffers { canvas_id, buffers });
}

#[op2(fast)]
pub fn op_read_buffer(state: &mut OpState, #[smi] canvas_id: u32, #[smi] src: u32) {
    queue_gl_fire_and_forget(state, GLCmd::ReadBuffer { canvas_id, src });
}

// ---- WebGL 2 Query objects ---------------------------------------

#[op2(fast)]
pub fn op_create_query(state: &mut OpState, #[smi] canvas_id: u32, #[smi] client_id: u32) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::CreateQuery {
            canvas_id,
            client_id,
        },
    );
}

#[op2(fast)]
pub fn op_delete_query(state: &mut OpState, #[smi] query: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DeleteQuery { query });
}

#[op2(fast)]
pub fn op_begin_query(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] query: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::BeginQuery {
            canvas_id,
            target,
            query,
        },
    );
}

#[op2(fast)]
pub fn op_end_query(state: &mut OpState, #[smi] canvas_id: u32, #[smi] target: u32) {
    queue_gl_fire_and_forget(state, GLCmd::EndQuery { canvas_id, target });
}

/// `getQueryParameter(query, pname)` - synchronous barrier because
/// callers poll `QUERY_RESULT_AVAILABLE` in a tight loop before
/// reading `QUERY_RESULT`; a queued call would stall behind normal
/// render traffic.
#[op2(fast)]
#[smi]
pub fn op_get_query_parameter(state: &mut OpState, #[smi] query: u32, #[smi] pname: u32) -> u32 {
    send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetQueryParameter { query, pname, resp })
    })
    .unwrap_or(0)
}

// ---- WebGL 2 Transform Feedback ----------------------------------

#[op2(fast)]
pub fn op_create_transform_feedback(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] client_id: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::CreateTransformFeedback {
            canvas_id,
            client_id,
        },
    );
}

#[op2(fast)]
pub fn op_delete_transform_feedback(state: &mut OpState, #[smi] tf: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DeleteTransformFeedback { tf });
}

#[op2(fast)]
pub fn op_bind_transform_feedback(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] tf: u32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::BindTransformFeedback {
            canvas_id,
            target,
            tf: if tf == 0 { None } else { Some(tf) },
        },
    );
}

#[op2(fast)]
pub fn op_begin_transform_feedback(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] primitive_mode: u32,
) {
    crate::rendering::webgl::error_state::set_transform_feedback_active(state, canvas_id, true);
    queue_gl_fire_and_forget(
        state,
        GLCmd::BeginTransformFeedback {
            canvas_id,
            primitive_mode,
        },
    );
}

#[op2(fast)]
pub fn op_end_transform_feedback(state: &mut OpState, #[smi] canvas_id: u32) {
    crate::rendering::webgl::error_state::set_transform_feedback_active(state, canvas_id, false);
    queue_gl_fire_and_forget(state, GLCmd::EndTransformFeedback { canvas_id });
}

#[op2(fast)]
pub fn op_pause_transform_feedback(state: &mut OpState, #[smi] canvas_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::PauseTransformFeedback { canvas_id });
}

#[op2(fast)]
pub fn op_resume_transform_feedback(state: &mut OpState, #[smi] canvas_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::ResumeTransformFeedback { canvas_id });
}

#[op2]
#[string]
pub fn op_get_transform_feedback_varying(
    state: &mut OpState,
    #[smi] program: u32,
    #[smi] index: u32,
) -> String {
    let info = send_gl_sync_with_flush(state, |resp| {
        RenderCommand::GL(GLCmd::GetTransformFeedbackVarying {
            program,
            index,
            resp,
        })
    })
    .ok()
    .flatten();

    if let Some((name, size, type_)) = info {
        let escaped_name = escape_for_json_string(&name);
        return format!(
            "{{\"name\":\"{}\",\"size\":{},\"type\":{}}}",
            escaped_name, size, type_
        );
    }

    String::new()
}

/// `transformFeedbackVaryings(program, varyings, bufferMode)`.
///
/// The JS shim passes the varyings array joined by `\x1f` (ASCII
/// Unit Separator) so the op2 fast lane can accept a single
/// `String` rather than spinning up a JSON parser.  US is invalid
/// in GLSL identifiers, so the split is unambiguous.
#[op2(fast)]
pub fn op_transform_feedback_varyings(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] program: u32,
    #[string] varyings_joined: String,
    #[smi] buffer_mode: u32,
) {
    let varyings: Vec<String> = if varyings_joined.is_empty() {
        Vec::new()
    } else {
        varyings_joined
            .split('\x1f')
            .map(|s| s.to_owned())
            .collect()
    };
    queue_gl_fire_and_forget(
        state,
        GLCmd::TransformFeedbackVaryings {
            canvas_id,
            program,
            varyings,
            buffer_mode,
        },
    );
}

// ---- WebGL 2 3D textures -----------------------------------------

fn normalize_tex_upload_3d_source(
    pixels: Option<&[u8]>,
    src_offset: u32,
    bytes_per_element: u32,
    pbo_offset: Option<u32>,
) -> shared::protocol::render_cmd::TexImage3DSource {
    if let Some(offset) = pbo_offset {
        return shared::protocol::render_cmd::TexImage3DSource::BufferOffset(offset);
    }
    let Some(pixels) = pixels else {
        return shared::protocol::render_cmd::TexImage3DSource::None;
    };
    let elem_bytes = usize::try_from(bytes_per_element.max(1)).unwrap_or(1);
    let start = elem_bytes.saturating_mul(src_offset as usize);
    let bytes = pixels.get(start..).unwrap_or(&[]);
    shared::protocol::render_cmd::TexImage3DSource::Bytes(Arc::new(bytes.to_vec()))
}

#[op2]
#[allow(clippy::too_many_arguments)]
pub fn op_tex_image_3d(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] internal_format: i32,
    #[smi] width: i32,
    #[smi] height: i32,
    #[smi] depth: i32,
    #[smi] border: i32,
    #[smi] format: u32,
    #[smi] ty: u32,
    // `None` when the call reserves storage without data.
    #[buffer] pixels: Option<&[u8]>,
    #[smi] src_offset: u32,
    #[smi] bytes_per_element: u32,
    #[smi] pbo_offset: i32,
) {
    let data = normalize_tex_upload_3d_source(
        pixels,
        src_offset,
        bytes_per_element,
        (pbo_offset >= 0).then_some(pbo_offset as u32),
    );
    queue_gl_fire_and_forget(
        state,
        GLCmd::TexImage3D {
            canvas_id,
            target,
            level,
            internal_format,
            width,
            height,
            depth,
            border,
            format,
            ty,
            data,
        },
    );
}

#[op2]
#[allow(clippy::too_many_arguments)]
pub fn op_tex_sub_image_3d(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] level: i32,
    #[smi] xoffset: i32,
    #[smi] yoffset: i32,
    #[smi] zoffset: i32,
    #[smi] width: i32,
    #[smi] height: i32,
    #[smi] depth: i32,
    #[smi] format: u32,
    #[smi] ty: u32,
    #[buffer] pixels: Option<&[u8]>,
    #[smi] src_offset: u32,
    #[smi] bytes_per_element: u32,
    #[smi] pbo_offset: i32,
) {
    let data = normalize_tex_upload_3d_source(
        pixels,
        src_offset,
        bytes_per_element,
        (pbo_offset >= 0).then_some(pbo_offset as u32),
    );
    queue_gl_fire_and_forget(
        state,
        GLCmd::TexSubImage3D {
            canvas_id,
            target,
            level,
            xoffset,
            yoffset,
            zoffset,
            width,
            height,
            depth,
            format,
            ty,
            data,
        },
    );
}

#[op2(fast)]
#[allow(clippy::too_many_arguments)]
pub fn op_tex_storage_3d(
    state: &mut OpState,
    #[smi] canvas_id: u32,
    #[smi] target: u32,
    #[smi] levels: i32,
    #[smi] internal_format: u32,
    #[smi] width: i32,
    #[smi] height: i32,
    #[smi] depth: i32,
) {
    queue_gl_fire_and_forget(
        state,
        GLCmd::TexStorage3D {
            canvas_id,
            target,
            levels,
            internal_format,
            width,
            height,
            depth,
        },
    );
}
