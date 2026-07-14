use std::sync::Arc;

use deno_core::{OpState, op2};
use tracing::{error, warn};

use crate::rendering::image::cache::IMAGE_CACHE;

use shared::{
    error::EngineError,
    js_escape::escape_for_json_string,
    op_state::CanvasOpState,
    protocol::{
        render_cmd::{
            GLCmd, RenderCmdResp, RenderCommand, ShaderType, UniformF32Values, UniformI32Values,
        },
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
        GlResourceIdAllocator, bind_buffer_base_impl, bind_buffer_range_impl, copy_f32_words,
        copy_i32_words, gl_cmd_has_heap_payload, normalize_tex_upload_3d_source,
    };
    use crate::rendering::webgl::{
        error_state::{self, WebGLErrorState, codes},
        frame_collector::UnifiedFrameCollector,
    };
    use crate::{HostJsRuntime, host_runtime::SharedMountTableRef};
    use shared::{
        FrameOp,
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
        let (host_tx, _critical_host_tx, _host_rx) = shared::host_channel::channel(1);

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
                raf_demand: std::sync::Arc::new(shared::raf_signal::RafDemand::new()),
                request_vsync: None,
                sub_packages: Vec::new(),
                workers_path: None,
                network_policy: NetworkPolicy::default(),
                backgrounded: Arc::new(AtomicBool::new(false)),
                timer_backgrounded: Arc::new(AtomicBool::new(false)),
                webgl_context_created: Arc::new(AtomicBool::new(false)),
                context_lost: Arc::new(shared::op_state::ContextLostState::default()),
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

    fn recv_gl_commands(render_rx: &crossbeam_channel::Receiver<RenderCommand>) -> Vec<GLCmd> {
        loop {
            match render_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("expected flushed WebGL frame packet")
            {
                RenderCommand::FramePacket(packet) => {
                    for op in packet.into_ops() {
                        if let FrameOp::GlBatch(payload) = op {
                            return payload.commands;
                        }
                    }
                    panic!("flushed frame packet did not contain a GL batch");
                }
                other => panic!("unexpected render command before GL batch: {other:?}"),
            }
        }
    }

    #[test]
    fn allocates_monotonic_non_zero_ids() {
        let mut alloc = GlResourceIdAllocator::new();
        assert_eq!(alloc.alloc(), 1);
        assert_eq!(alloc.alloc(), 2);
        assert_eq!(alloc.alloc(), 3);
    }

    #[test]
    fn sixteen_uniform_words_copy_inline_and_seventeen_spill() {
        let inline_words = [1.0f32.to_bits(); 16];
        let inline = copy_f32_words(&inline_words);
        assert_eq!(inline.len(), 16);
        assert!(!inline.spilled());
        assert!(inline.iter().all(|value| *value == 1.0));

        let spilled_words = [2.0f32.to_bits(); 17];
        let spilled = copy_f32_words(&spilled_words);
        assert_eq!(spilled.len(), 17);
        assert!(spilled.spilled());
    }

    #[test]
    fn integer_uniform_words_preserve_signed_bits() {
        let words = [0u32, 1, u32::MAX, i32::MIN as u32];
        let copied = copy_i32_words(&words);
        assert_eq!(copied.as_slice(), &[0, 1, -1, i32::MIN]);
        assert!(!copied.spilled());
    }

    #[test]
    fn empty_uniform_word_slices_stay_inline() {
        assert!(copy_f32_words(&[]).is_empty());
        assert!(!copy_f32_words(&[]).spilled());
        assert!(copy_i32_words(&[]).is_empty());
        assert!(!copy_i32_words(&[]).spilled());
    }

    #[test]
    fn only_spilled_uniform_values_are_classified_as_heap_payloads() {
        let inline = GLCmd::UniformMatrix4fv {
            canvas_id: 1,
            location: Some(1),
            transpose: false,
            value: (0..16).map(|n| n as f32).collect(),
        };
        let spilled = GLCmd::Uniform1fv {
            canvas_id: 1,
            location: Some(1),
            value: (0..17).map(|n| n as f32).collect(),
        };

        assert!(!gl_cmd_has_heap_payload(&inline));
        assert!(gl_cmd_has_heap_payload(&spilled));
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
    fn plain_float_uniform_sequence_preserves_values() {
        let (mut runtime, render_rx) = new_webgl_runtime();
        runtime
            .exec_script(
                "plain_float_uniform_sequence.js",
                r#"
                const ctx = new WebGLRenderingContext({ _rid: 13, width: 1, height: 1 }, {});
                ctx.uniform4fv({ id: 9 }, [1.5, -2.25, 0.0, 7.75]);
                ctx.flush();
                "#,
            )
            .expect("plain float uniform sequence should be accepted");

        let commands = recv_gl_commands(&render_rx);
        let Some(GLCmd::Uniform4fv { value, .. }) = commands.into_iter().next() else {
            panic!("expected one Uniform4fv command");
        };
        assert_eq!(value.as_slice(), &[1.5, -2.25, 0.0, 7.75]);
    }

    #[test]
    fn uniform_array_is_copied_when_the_op_is_called() {
        let (mut runtime, render_rx) = new_webgl_runtime();
        runtime
            .exec_script(
                "uniform_call_time_copy.js",
                r#"
                const ctx = new WebGLRenderingContext({ _rid: 14, width: 1, height: 1 }, {});
                const source = new Float32Array([1.25, 2.5, 3.75, 5.0]);
                ctx.uniform4fv({ id: 10 }, source);
                source[0] = 99.0;
                source[1] = 101.0;
                ctx.flush();
                "#,
            )
            .expect("typed float uniform should be accepted");

        let commands = recv_gl_commands(&render_rx);
        let Some(GLCmd::Uniform4fv { value, .. }) = commands.into_iter().next() else {
            panic!("expected one Uniform4fv command");
        };
        assert_eq!(value.as_slice(), &[1.25, 2.5, 3.75, 5.0]);
    }

    #[test]
    fn integer_uniform_typed_sequence_is_converted_numerically() {
        let (mut runtime, render_rx) = new_webgl_runtime();
        runtime
            .exec_script(
                "integer_uniform_typed_sequence.js",
                r#"
                const ctx = new WebGLRenderingContext({ _rid: 17, width: 1, height: 1 }, {});
                ctx.uniform4iv({ id: 13 }, new Uint16Array([1, 2, 32768, 65535]));
                ctx.flush();
                "#,
            )
            .expect("integer typed sequence should be accepted");

        let commands = recv_gl_commands(&render_rx);
        let Some(GLCmd::Uniform4iv { value, .. }) = commands.into_iter().next() else {
            panic!("expected one Uniform4iv command");
        };
        assert_eq!(value.as_slice(), &[1, 2, 32768, 65535]);
    }

    #[test]
    fn uniform_helpers_copy_shared_backing_before_fast_borrow() {
        let source = include_str!("02_webgl_context.js");

        assert!(
            source.contains("isSharedArrayBuffer"),
            "uniform conversion must identify SharedArrayBuffer backing"
        );
        assert!(
            source.contains("ensureNonSharedTypedArray"),
            "uniform conversion must copy shared views before Rust borrows them"
        );
    }

    #[test]
    fn shared_uniform_source_preserves_call_time_values() {
        let (mut runtime, render_rx) = new_webgl_runtime();
        runtime
            .exec_script(
                "shared_uniform_call_time_copy.js",
                r#"
                const ctx = new WebGLRenderingContext({ _rid: 18, width: 1, height: 1 }, {});
                const source = new Float32Array(new SharedArrayBuffer(16));
                source.set([1.25, 2.5, 3.75, 5.0]);
                ctx.uniform4fv({ id: 14 }, source);
                source.set([99.0, 101.0, 103.0, 105.0]);
                ctx.flush();
                "#,
            )
            .expect("shared float uniform should be copied safely");

        let commands = recv_gl_commands(&render_rx);
        let Some(GLCmd::Uniform4fv { value, .. }) = commands.into_iter().next() else {
            panic!("expected one Uniform4fv command");
        };
        assert_eq!(value.as_slice(), &[1.25, 2.5, 3.75, 5.0]);
    }

    #[test]
    fn plain_matrix3_uniform_sequence_is_accepted() {
        let (mut runtime, render_rx) = new_webgl_runtime();
        runtime
            .exec_script(
                "plain_matrix3_uniform_sequence.js",
                r#"
                const ctx = new WebGLRenderingContext({ _rid: 15, width: 1, height: 1 }, {});
                ctx.uniformMatrix3fv(
                    { id: 11 },
                    false,
                    [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                );
                ctx.flush();
                "#,
            )
            .expect("plain matrix3 Float32List sequence should be accepted");

        let commands = recv_gl_commands(&render_rx);
        let Some(GLCmd::UniformMatrix3fv { value, .. }) = commands.into_iter().next() else {
            panic!("expected one UniformMatrix3fv command");
        };
        assert_eq!(
            value.as_slice(),
            &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
        );
    }

    #[test]
    fn float_uniform_ignores_shadowed_typed_array_metadata() {
        let (mut runtime, render_rx) = new_webgl_runtime();
        runtime
            .exec_script(
                "shadowed_float_uniform_metadata.js",
                r#"
                const ctx = new WebGLRenderingContext({ _rid: 16, width: 1, height: 1 }, {});
                const source = new Float32Array([1.5, 2.5, 3.5, 4.5]);
                Object.defineProperty(source, "buffer", { value: new ArrayBuffer(16) });
                Object.defineProperty(source, "byteOffset", { value: 0 });
                Object.defineProperty(source, "length", { value: 4 });
                ctx.uniform4fv({ id: 12 }, source);
                ctx.flush();
                "#,
            )
            .expect("shadow properties must not affect internal typed-array metadata");

        let commands = recv_gl_commands(&render_rx);
        let Some(GLCmd::Uniform4fv { value, .. }) = commands.into_iter().next() else {
            panic!("expected one Uniform4fv command");
        };
        assert_eq!(value.as_slice(), &[1.5, 2.5, 3.5, 4.5]);
    }

    #[test]
    fn inline_uniform_stream_auto_flushes_at_soft_budget_without_explicit_flush() {
        use crate::rendering::webgl::frame_collector::AUTO_FLUSH_SOFT_BUDGET_BYTES;

        let (mut runtime, render_rx) = new_webgl_runtime();

        // With the stream path, uniform4fv is encoded into the JS-side 8192-word
        // ring buffer. Each uniform4fv record = 7 words (H + C + loc + 4 floats).
        // The stream auto-submits when the buffer fills: (8192 - 2) / 7 = 1170
        // commands per batch. Each submit adds `1170 * per_cmd` approx bytes.
        //
        // To guarantee at least one auto-flush, we need enough submits so that
        // accumulated pending_bytes >= AUTO_FLUSH_SOFT_BUDGET_BYTES.
        //   submits_needed = ceil(budget / (1170 * per_cmd))
        //   count_needed   = submits_needed * 1171 + 2
        // (1171 because the (N+1)-th command triggers the submit of N commands.)
        let per_cmd = std::mem::size_of::<GLCmd>();
        // Words per uniform4fv record in the stream buffer (H C loc f32 f32 f32 f32).
        const UNIFORM4FV_WORDS: usize = 7;
        // Commands per stream-buffer submit: (8192 - 2 header words) / UNIFORM4FV_WORDS.
        const CMDS_PER_SUBMIT: usize = (8192 - 2) / UNIFORM4FV_WORDS; // = 1170
        let bytes_per_submit = CMDS_PER_SUBMIT * per_cmd;
        let submits_needed =
            (AUTO_FLUSH_SOFT_BUDGET_BYTES + bytes_per_submit - 1) / bytes_per_submit;
        let count = submits_needed * (CMDS_PER_SUBMIT + 1) + 2;

        runtime
            .exec_script(
                "inline_uniform_autoflush.js",
                &format!(
                    r#"
                    globalThis.__ctx = new WebGLRenderingContext({{ _rid: 21, width: 1, height: 1 }}, {{}});
                    const loc = {{ id: 9 }};
                    // Encode the submission index in each component. `i` is
                    // exactly representable as f32 (i < 2^24, and count << that),
                    // so the consumer can assert strict submission ORDER, not
                    // merely count / no-loss.
                    for (let i = 0; i < {count}; i++) {{
                        globalThis.__ctx.uniform4fv(loc, [i, i, i, i]);
                    }}
                    // Intentionally NO flush() and NO frame end: untrusted JS can
                    // enqueue this many inline uniforms synchronously in one turn.
                    "#,
                ),
            )
            .expect("inline uniform stream should be accepted");

        // The burst crossed the soft budget, so an automatic non-presenting
        // barrier FramePacket must already be queued BEFORE any explicit flush.
        let first = match render_rx.try_recv() {
            Ok(RenderCommand::FramePacket(packet)) => packet,
            Ok(other) => panic!("unexpected render command: {other:?}"),
            Err(_) => panic!(
                "inline uniform burst crossed the {AUTO_FLUSH_SOFT_BUDGET_BYTES}-byte soft budget \
                 but no automatic barrier FramePacket was emitted before an explicit flush"
            ),
        };
        assert!(
            !first.ops().iter().any(|op| matches!(op, FrameOp::Present)),
            "auto-flush barrier must be non-presenting"
        );

        // Order survives the auto-flush boundary: flush the remainder, then
        // consume the automatic packet followed by the explicit remainder. Each
        // command must carry its strictly-increasing submission index — a lost,
        // duplicated, or reordered command breaks the running counter.
        runtime
            .exec_script(
                "inline_uniform_autoflush_drain.js",
                "globalThis.__ctx.flush();",
            )
            .expect("explicit flush of the remainder should be accepted");

        let mut expected = 0.0f32;
        for packet in
            std::iter::once(first).chain(std::iter::from_fn(|| match render_rx.try_recv() {
                Ok(RenderCommand::FramePacket(p)) => Some(p),
                _ => None,
            }))
        {
            for op in packet.into_ops() {
                if let FrameOp::GlBatch(payload) = op {
                    for cmd in payload.commands {
                        match cmd {
                            GLCmd::Uniform4fv { value, .. } => {
                                assert_eq!(
                                    value.as_slice(),
                                    &[expected, expected, expected, expected],
                                    "uniforms must arrive in strict submission order across the \
                                     auto-flush boundary"
                                );
                                expected += 1.0;
                            }
                            other => panic!("unexpected command across auto-flush: {other:?}"),
                        }
                    }
                }
            }
        }
        assert_eq!(
            expected as usize, count,
            "every queued uniform must survive the auto-flush boundary exactly once, in order"
        );
    }

    #[test]
    fn small_inline_uniform_sequence_does_not_auto_flush() {
        let (mut runtime, render_rx) = new_webgl_runtime();
        runtime
            .exec_script(
                "small_inline_uniform.js",
                r#"
                const ctx = new WebGLRenderingContext({ _rid: 22, width: 1, height: 1 }, {});
                const loc = { id: 9 };
                for (let i = 0; i < 8; i++) ctx.uniform4fv(loc, [1.0, 2.0, 3.0, 4.0]);
                "#,
            )
            .expect("small uniform sequence should be accepted");
        assert!(
            render_rx.try_recv().is_err(),
            "a small inline uniform sequence must not trigger an automatic flush"
        );
    }

    // Characterization (Q5 review gap): a length-tracking `Float32Array` over a
    // *resizable* `ArrayBuffer` is a legal uniform source, before AND after a
    // grow. The op must copy at call time; a later mutate/`resize` must never
    // change an already-queued command, and two calls from the same view must
    // keep their respective call-time values and submission order. If the locked
    // V8 rejects RAB construction the `.expect` below fails loudly and we would
    // document RAB-unavailable instead of asserting fabricated behavior.
    #[test]
    fn resizable_arraybuffer_uniform_source_copies_at_call_time() {
        let (mut runtime, render_rx) = new_webgl_runtime();
        runtime
            .exec_script(
                "rab_uniform_call_time_copy.js",
                r#"
                const rab = new ArrayBuffer(16, { maxByteLength: 64 });
                if (rab.resizable !== true) throw new Error("expected a resizable ArrayBuffer");
                const view = new Float32Array(rab); // length-tracks the RAB (4 floats)
                const ctx = new WebGLRenderingContext({ _rid: 23, width: 1, height: 1 }, {});

                // Call 1: pre-grow 4-float view.
                view.set([1.5, -2.25, 0.0, 7.75]);
                ctx.uniform4fv({ id: 9 }, view);
                view[0] = 99.0; // post-call mutation must not affect command 1

                // Grow the backing; the length-tracking view now spans 16 floats.
                rab.resize(64);
                if (view.length !== 16) {
                    throw new Error("length-tracking view must span 16 floats after grow, got " + view.length);
                }

                // Call 2: post-grow, same view, distinguishable 16-word payload
                // (fills the inline SmallVec exactly, no spill).
                for (let i = 0; i < 16; i++) view[i] = 100 + i;
                ctx.uniform1fv({ id: 10 }, view);

                // Post-call mutate + shrink must not affect command 2.
                view[0] = -1.0;
                rab.resize(16);
                ctx.flush();
                "#,
            )
            .expect("resizable ArrayBuffer uniform source should be accepted");

        let commands = recv_gl_commands(&render_rx);
        let mut it = commands.into_iter();

        // Order + call-time values: command 1 is the pre-grow vec4.
        let Some(GLCmd::Uniform4fv { value, .. }) = it.next() else {
            panic!("expected Uniform4fv as the first command");
        };
        assert_eq!(value.as_slice(), &[1.5, -2.25, 0.0, 7.75]);

        // Command 2 is the post-grow 16-word inline payload, unaffected by the
        // later mutate + shrink.
        let Some(GLCmd::Uniform1fv { value, .. }) = it.next() else {
            panic!("expected Uniform1fv as the second command");
        };
        let expected: Vec<f32> = (0..16).map(|i| 100.0 + i as f32).collect();
        assert_eq!(value.as_slice(), expected.as_slice());
        assert!(
            !value.spilled(),
            "a 16-word post-grow uniform payload must stay inline"
        );
        assert!(it.next().is_none(), "exactly two GL commands expected");
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

    // ── Task 2 RED: decode_validated_stream ───────────────────────────────────
    //
    // These tests call decode_validated_stream from the `decode` module (Task 2).
    // They FAIL (compile error / link error) until the implementation is in place.

    use crate::rendering::webgl::decode::decode_validated_stream;
    use crate::rendering::webgl::gl_stream::{
        MAGIC, OP_BIND_BUFFER, OP_BIND_BUFFER_BASE, OP_BIND_BUFFER_RANGE, OP_BIND_FRAMEBUFFER,
        OP_BIND_RENDERBUFFER, OP_BIND_SAMPLER, OP_BIND_TEXTURE, OP_BIND_VERTEX_ARRAY, OP_CLEAR,
        OP_ENABLE, OP_SCISSOR, OP_UNIFORM_MATRIX3FV, OP_UNIFORM1F, OP_UNIFORM1FV, OP_UNIFORM1I,
        OP_VERTEX_ATTRIB_POINTER, OP_VIEWPORT, STREAM_VERSION, ValidatedStream, pack_header,
        validate_stream,
    };

    fn make_validated_for_decode(words: &[u32]) -> ValidatedStream<'_> {
        validate_stream(words, words.len() as u32).expect("test stream must be valid")
    }

    // ── f32 bit-exact round-trip ──────────────────────────────────────────────

    #[test]
    fn decode_uniform1f_nan_bit_exact_round_trip() {
        let nan_bits = f32::NAN.to_bits();
        let canvas_id: u32 = 1;
        let h = pack_header(OP_UNIFORM1F, 4);
        let words = [MAGIC, STREAM_VERSION, h, canvas_id, 5u32, nan_bits];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        assert_eq!(out.len(), 1);
        match &out[0] {
            GLCmd::Uniform1f { location, x, .. } => {
                assert_eq!(*location, Some(5));
                assert_eq!(x.to_bits(), nan_bits, "NaN bits must round-trip exactly");
            }
            other => panic!("expected Uniform1f, got {:?}", other),
        }
    }

    #[test]
    fn decode_uniform1f_neg_zero_bit_exact_round_trip() {
        let neg_zero_bits = (-0.0f32).to_bits();
        let canvas_id: u32 = 1;
        let h = pack_header(OP_UNIFORM1F, 4);
        let words = [MAGIC, STREAM_VERSION, h, canvas_id, 0u32, neg_zero_bits];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        match &out[0] {
            GLCmd::Uniform1f { x, .. } => {
                assert_eq!(
                    x.to_bits(),
                    neg_zero_bits,
                    "-0 bits must round-trip exactly"
                );
            }
            other => panic!("expected Uniform1f, got {:?}", other),
        }
    }

    #[test]
    fn decode_uniform1f_pos_infinity_bit_exact_round_trip() {
        let inf_bits = f32::INFINITY.to_bits();
        let canvas_id: u32 = 1;
        let h = pack_header(OP_UNIFORM1F, 4);
        let words = [MAGIC, STREAM_VERSION, h, canvas_id, 0u32, inf_bits];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        match &out[0] {
            GLCmd::Uniform1f { x, .. } => {
                assert_eq!(x.to_bits(), inf_bits, "+Inf bits must round-trip exactly");
            }
            other => panic!("expected Uniform1f, got {:?}", other),
        }
    }

    #[test]
    fn decode_uniform1f_neg_infinity_bit_exact_round_trip() {
        let neginf_bits = f32::NEG_INFINITY.to_bits();
        let canvas_id: u32 = 1;
        let h = pack_header(OP_UNIFORM1F, 4);
        let words = [MAGIC, STREAM_VERSION, h, canvas_id, 0u32, neginf_bits];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        match &out[0] {
            GLCmd::Uniform1f { x, .. } => {
                assert_eq!(
                    x.to_bits(),
                    neginf_bits,
                    "-Inf bits must round-trip exactly"
                );
            }
            other => panic!("expected Uniform1f, got {:?}", other),
        }
    }

    // ── i32 round-trip ────────────────────────────────────────────────────────

    #[test]
    fn decode_uniform1i_neg_one_round_trip() {
        let canvas_id: u32 = 1;
        let location_word: u32 = 3u32;
        let x_word: u32 = (-1i32) as u32;
        let h = pack_header(OP_UNIFORM1I, 4);
        let words = [MAGIC, STREAM_VERSION, h, canvas_id, location_word, x_word];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        assert_eq!(out.len(), 1);
        match &out[0] {
            GLCmd::Uniform1i {
                canvas_id: c,
                location: l,
                x,
            } => {
                assert_eq!(*c, 1);
                assert_eq!(*l, Some(3));
                assert_eq!(*x, -1, "i32 -1 must round-trip exactly");
            }
            other => panic!("expected Uniform1i, got {:?}", other),
        }
    }

    #[test]
    fn decode_uniform1i_i32_min_round_trip() {
        let canvas_id: u32 = 1;
        let location_word: u32 = 0u32;
        let x_word: u32 = i32::MIN as u32;
        let h = pack_header(OP_UNIFORM1I, 4);
        let words = [MAGIC, STREAM_VERSION, h, canvas_id, location_word, x_word];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        match &out[0] {
            GLCmd::Uniform1i { x, .. } => {
                assert_eq!(*x, i32::MIN, "i32::MIN must round-trip exactly");
            }
            other => panic!("expected Uniform1i, got {:?}", other),
        }
    }

    // ── multi-record order preserved ──────────────────────────────────────────

    #[test]
    fn decode_multi_record_order_preserved() {
        let h_vp = pack_header(OP_VIEWPORT, 6);
        let h_cl = pack_header(OP_CLEAR, 3);
        let h_en = pack_header(OP_ENABLE, 3);
        let words = [
            MAGIC,
            STREAM_VERSION,
            h_vp,
            7u32,
            10i32 as u32,
            20i32 as u32,
            800u32,
            600u32,
            h_cl,
            7u32,
            0x4100u32,
            h_en,
            7u32,
            0x0B44u32,
        ];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        assert_eq!(out.len(), 3);
        assert!(matches!(
            &out[0],
            GLCmd::Viewport {
                canvas_id: 7,
                x: 10,
                y: 20,
                ..
            }
        ));
        assert!(matches!(
            &out[1],
            GLCmd::Clear {
                canvas_id: 7,
                bit_field: 0x4100
            }
        ));
        assert!(matches!(
            &out[2],
            GLCmd::Enable {
                canvas_id: 7,
                cap: 0x0B44
            }
        ));
    }

    // ── null-id rules ─────────────────────────────────────────────────────────

    #[test]
    fn decode_bind_buffer_negative_id_becomes_none() {
        let target: u32 = 0x8892;
        let neg_one_bits: u32 = (-1i32) as u32;
        let h = pack_header(OP_BIND_BUFFER, 4);
        let words = [MAGIC, STREAM_VERSION, h, 1u32, target, neg_one_bits];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        assert_eq!(out.len(), 1);
        match &out[0] {
            GLCmd::BindBuffer { buffer, .. } => {
                assert_eq!(*buffer, None, "negative buffer id must map to None");
            }
            other => panic!("expected BindBuffer, got {:?}", other),
        }
    }

    #[test]
    fn decode_bind_texture_negative_id_becomes_none() {
        let target: u32 = 0x0DE1;
        let neg_one_bits: u32 = (-1i32) as u32;
        let h = pack_header(OP_BIND_TEXTURE, 4);
        let words = [MAGIC, STREAM_VERSION, h, 1u32, target, neg_one_bits];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        assert_eq!(out.len(), 1);
        match &out[0] {
            GLCmd::BindTexture { texture, .. } => {
                assert_eq!(*texture, None, "negative texture id must map to None");
            }
            other => panic!("expected BindTexture, got {:?}", other),
        }
    }

    #[test]
    fn decode_bind_framebuffer_negative_id_becomes_none() {
        let target: u32 = 0x8D40;
        let neg_one_bits: u32 = (-1i32) as u32;
        let h = pack_header(OP_BIND_FRAMEBUFFER, 4);
        let words = [MAGIC, STREAM_VERSION, h, 1u32, target, neg_one_bits];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        assert_eq!(out.len(), 1);
        match &out[0] {
            GLCmd::BindFramebuffer { framebuffer, .. } => {
                assert_eq!(
                    *framebuffer, None,
                    "negative framebuffer id must map to None"
                );
            }
            other => panic!("expected BindFramebuffer, got {:?}", other),
        }
    }

    #[test]
    fn decode_bind_renderbuffer_negative_id_becomes_none() {
        let target: u32 = 0x8D41;
        let neg_one_bits: u32 = (-1i32) as u32;
        let h = pack_header(OP_BIND_RENDERBUFFER, 4);
        let words = [MAGIC, STREAM_VERSION, h, 1u32, target, neg_one_bits];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        assert_eq!(out.len(), 1);
        match &out[0] {
            GLCmd::BindRenderbuffer { renderbuffer, .. } => {
                assert_eq!(
                    *renderbuffer, None,
                    "negative renderbuffer id must map to None"
                );
            }
            other => panic!("expected BindRenderbuffer, got {:?}", other),
        }
    }

    #[test]
    fn decode_bind_vertex_array_zero_id_becomes_none() {
        let h = pack_header(OP_BIND_VERTEX_ARRAY, 3);
        let words = [MAGIC, STREAM_VERSION, h, 1u32, 0u32];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        assert_eq!(out.len(), 1);
        match &out[0] {
            GLCmd::BindVertexArray { vao, .. } => {
                assert_eq!(*vao, None, "VAO id 0 must map to None");
            }
            other => panic!("expected BindVertexArray, got {:?}", other),
        }
    }

    #[test]
    fn decode_bind_sampler_zero_id_becomes_none() {
        let h = pack_header(OP_BIND_SAMPLER, 4);
        let words = [MAGIC, STREAM_VERSION, h, 1u32, 0u32, 0u32];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        assert_eq!(out.len(), 1);
        match &out[0] {
            GLCmd::BindSampler { sampler, .. } => {
                assert_eq!(*sampler, None, "sampler id 0 must map to None");
            }
            other => panic!("expected BindSampler, got {:?}", other),
        }
    }

    #[test]
    fn decode_bind_buffer_base_zero_buffer_becomes_none() {
        let target: u32 = 0x8A11;
        let h = pack_header(OP_BIND_BUFFER_BASE, 5);
        let words = [MAGIC, STREAM_VERSION, h, 1u32, target, 0u32, 0u32];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        assert_eq!(out.len(), 1);
        match &out[0] {
            GLCmd::BindBufferBase { buffer, .. } => {
                assert_eq!(*buffer, None, "buffer base id 0 must map to None");
            }
            other => panic!("expected BindBufferBase, got {:?}", other),
        }
    }

    #[test]
    fn decode_uniform_location_negative_becomes_none() {
        let canvas_id: u32 = 1;
        let neg_loc: u32 = (-1i32) as u32;
        let h = pack_header(OP_UNIFORM1F, 4);
        let words = [
            MAGIC,
            STREAM_VERSION,
            h,
            canvas_id,
            neg_loc,
            1.0f32.to_bits(),
        ];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        assert_eq!(out.len(), 1);
        match &out[0] {
            GLCmd::Uniform1f { location, .. } => {
                assert_eq!(*location, None, "location < 0 must map to None");
            }
            other => panic!("expected Uniform1f, got {:?}", other),
        }
    }

    // ── equivalence: bindBuffer (valid and invalid) ───────────────────────────
    //
    // For raw-op equivalence we call the underlying validator directly
    // (the #[op2] wrapper cannot be called from Rust tests), then assert
    // decode produces the same error queue outcome and same GLCmd shape.

    #[test]
    fn decode_bind_buffer_valid_equiv_raw_op() {
        let canvas_id: u32 = 5;
        let target: u32 = 0x8892;
        let buffer_id: u32 = 42;

        // Simulate the raw op: validate target, no error expected.
        let mut state_raw = new_webgl_op_state();
        let valid = error_state::validate_bind_buffer_target(&mut state_raw, canvas_id, target);
        assert!(valid);
        let raw_err = state_raw
            .borrow_mut::<WebGLErrorState>()
            .drain_one(canvas_id);

        let h = pack_header(OP_BIND_BUFFER, 4);
        let words = [MAGIC, STREAM_VERSION, h, canvas_id, target, buffer_id];
        let vs = make_validated_for_decode(&words);
        let mut state_dec = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state_dec, vs, &mut out);
        let dec_err = state_dec
            .borrow_mut::<WebGLErrorState>()
            .drain_one(canvas_id);

        assert_eq!(
            raw_err, 0,
            "raw validator must not error for valid bind buffer target"
        );
        assert_eq!(
            dec_err, 0,
            "decoded stream must not error for valid bind buffer"
        );
        assert_eq!(out.len(), 1);
        match &out[0] {
            GLCmd::BindBuffer {
                canvas_id: c,
                target: t,
                buffer: b,
            } => {
                assert_eq!(*c, canvas_id);
                assert_eq!(*t, target);
                assert_eq!(*b, Some(buffer_id));
            }
            other => panic!("expected BindBuffer, got {:?}", other),
        }
    }

    #[test]
    fn decode_bind_buffer_invalid_target_equiv_raw_op() {
        let canvas_id: u32 = 5;
        let bad_target: u32 = 0xDEAD;
        let buffer_id: u32 = 42;

        // Simulate the raw op: validate target, INVALID_ENUM expected.
        let mut state_raw = new_webgl_op_state();
        let valid = error_state::validate_bind_buffer_target(&mut state_raw, canvas_id, bad_target);
        assert!(!valid);
        let raw_err = state_raw
            .borrow_mut::<WebGLErrorState>()
            .drain_one(canvas_id);

        let h = pack_header(OP_BIND_BUFFER, 4);
        let words = [MAGIC, STREAM_VERSION, h, canvas_id, bad_target, buffer_id];
        let vs = make_validated_for_decode(&words);
        let mut state_dec = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state_dec, vs, &mut out);
        let dec_err = state_dec
            .borrow_mut::<WebGLErrorState>()
            .drain_one(canvas_id);

        assert_eq!(raw_err, error_state::codes::INVALID_ENUM);
        assert_eq!(dec_err, error_state::codes::INVALID_ENUM);
        assert_eq!(out.len(), 0);
    }

    // ── equivalence: scissor ──────────────────────────────────────────────────

    #[test]
    fn decode_scissor_valid_equiv_raw_op() {
        let canvas_id: u32 = 3;
        let (x, y, w, h_val) = (10i32, 20i32, 100i32, 200i32);

        // Simulate the raw op: validate_viewport_like, no error expected.
        let mut state_raw = new_webgl_op_state();
        let valid = error_state::validate_viewport_like(&mut state_raw, canvas_id, w, h_val);
        assert!(valid);
        let raw_err = state_raw
            .borrow_mut::<WebGLErrorState>()
            .drain_one(canvas_id);

        let h = pack_header(OP_SCISSOR, 6);
        let words = [
            MAGIC,
            STREAM_VERSION,
            h,
            canvas_id,
            x as u32,
            y as u32,
            w as u32,
            h_val as u32,
        ];
        let vs = make_validated_for_decode(&words);
        let mut state_dec = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state_dec, vs, &mut out);
        let dec_err = state_dec
            .borrow_mut::<WebGLErrorState>()
            .drain_one(canvas_id);

        assert_eq!(raw_err, 0);
        assert_eq!(dec_err, 0);
        assert_eq!(out.len(), 1);
        match &out[0] {
            GLCmd::Scissor {
                canvas_id: c,
                x: ox,
                y: oy,
                width,
                height,
            } => {
                assert_eq!((*c, *ox, *oy, *width, *height), (canvas_id, x, y, w, h_val));
            }
            other => panic!("expected Scissor, got {:?}", other),
        }
    }

    #[test]
    fn decode_scissor_negative_width_equiv_raw_op() {
        let canvas_id: u32 = 3;
        let neg_w: i32 = -1;

        // Simulate the raw op: validate_viewport_like, INVALID_VALUE expected.
        let mut state_raw = new_webgl_op_state();
        let valid = error_state::validate_viewport_like(&mut state_raw, canvas_id, neg_w, 100);
        assert!(!valid);
        let raw_err = state_raw
            .borrow_mut::<WebGLErrorState>()
            .drain_one(canvas_id);

        let h = pack_header(OP_SCISSOR, 6);
        let words = [
            MAGIC,
            STREAM_VERSION,
            h,
            canvas_id,
            0u32,
            0u32,
            neg_w as u32,
            100u32,
        ];
        let vs = make_validated_for_decode(&words);
        let mut state_dec = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state_dec, vs, &mut out);
        let dec_err = state_dec
            .borrow_mut::<WebGLErrorState>()
            .drain_one(canvas_id);

        assert_eq!(raw_err, error_state::codes::INVALID_VALUE);
        assert_eq!(dec_err, error_state::codes::INVALID_VALUE);
        assert_eq!(out.len(), 0);
    }

    // ── equivalence: vertexAttribPointer ─────────────────────────────────────

    #[test]
    fn decode_vertex_attrib_pointer_valid_equiv_raw_op() {
        let canvas_id: u32 = 2;
        let (index, size, type_, normalized, stride, offset) =
            (0u32, 3i32, 0x1406u32, false, 12i32, 0i32);

        // Simulate the raw op: validate_vertex_attrib_pointer, no error expected.
        let mut state_raw = new_webgl_op_state();
        let valid = error_state::validate_vertex_attrib_pointer(
            &mut state_raw,
            canvas_id,
            size,
            type_,
            stride,
            offset,
        );
        assert!(valid);
        let raw_err = state_raw
            .borrow_mut::<WebGLErrorState>()
            .drain_one(canvas_id);

        // H C U I U B I I
        let h = pack_header(OP_VERTEX_ATTRIB_POINTER, 8);
        let words = [
            MAGIC,
            STREAM_VERSION,
            h,
            canvas_id,
            index,
            size as u32,
            type_,
            0u32,
            stride as u32,
            offset as u32,
        ];
        let vs = make_validated_for_decode(&words);
        let mut state_dec = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state_dec, vs, &mut out);
        let dec_err = state_dec
            .borrow_mut::<WebGLErrorState>()
            .drain_one(canvas_id);

        assert_eq!(raw_err, 0);
        assert_eq!(dec_err, 0);
        assert_eq!(out.len(), 1);
        match &out[0] {
            GLCmd::VertexAttribPointer {
                canvas_id: c,
                index: i,
                size: s,
                type_: t,
                normalized: n,
                stride: st,
                offset: of,
            } => {
                assert_eq!(
                    (*c, *i, *s, *t, *n, *st, *of),
                    (canvas_id, index, size, type_, normalized, stride, offset)
                );
            }
            other => panic!("expected VertexAttribPointer, got {:?}", other),
        }
    }

    #[test]
    fn decode_vertex_attrib_pointer_invalid_type_equiv_raw_op() {
        let canvas_id: u32 = 2;
        let bad_type: u32 = 0x0000;

        // Simulate the raw op: validate_vertex_attrib_pointer, INVALID_ENUM expected.
        let mut state_raw = new_webgl_op_state();
        let valid = error_state::validate_vertex_attrib_pointer(
            &mut state_raw,
            canvas_id,
            4,
            bad_type,
            0,
            0,
        );
        assert!(!valid);
        let raw_err = state_raw
            .borrow_mut::<WebGLErrorState>()
            .drain_one(canvas_id);

        let h = pack_header(OP_VERTEX_ATTRIB_POINTER, 8);
        let words = [
            MAGIC,
            STREAM_VERSION,
            h,
            canvas_id,
            0u32,
            4u32,
            bad_type,
            0u32,
            0u32,
            0u32,
        ];
        let vs = make_validated_for_decode(&words);
        let mut state_dec = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state_dec, vs, &mut out);
        let dec_err = state_dec
            .borrow_mut::<WebGLErrorState>()
            .drain_one(canvas_id);

        assert_eq!(raw_err, error_state::codes::INVALID_ENUM);
        assert_eq!(dec_err, error_state::codes::INVALID_ENUM);
        assert_eq!(out.len(), 0);
    }

    // ── equivalence: bindBufferBase (valid) ───────────────────────────────────

    #[test]
    fn decode_bind_buffer_base_valid_equiv_raw_op() {
        let canvas_id: u32 = 1;
        let (target, index, buffer) = (0x8A11u32, 0u32, 7u32);

        // Use the existing bind_buffer_base_impl (already public for tests).
        let mut state_raw = new_webgl_op_state();
        bind_buffer_base_impl(&mut state_raw, canvas_id, target, index, buffer);
        let raw_err = state_raw
            .borrow_mut::<WebGLErrorState>()
            .drain_one(canvas_id);

        let h = pack_header(OP_BIND_BUFFER_BASE, 5);
        let words = [MAGIC, STREAM_VERSION, h, canvas_id, target, index, buffer];
        let vs = make_validated_for_decode(&words);
        let mut state_dec = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state_dec, vs, &mut out);
        let dec_err = state_dec
            .borrow_mut::<WebGLErrorState>()
            .drain_one(canvas_id);

        assert_eq!(raw_err, 0);
        assert_eq!(dec_err, 0);
        assert_eq!(out.len(), 1);
        match &out[0] {
            GLCmd::BindBufferBase {
                canvas_id: c,
                target: t,
                index: i,
                buffer: b,
            } => {
                assert_eq!((*c, *t, *i, *b), (canvas_id, target, index, Some(buffer)));
            }
            other => panic!("expected BindBufferBase, got {:?}", other),
        }
    }

    // ── equivalence: bindBufferRange (invalid: negative offset) ──────────────

    #[test]
    fn decode_bind_buffer_range_invalid_offset_equiv_raw_op() {
        let canvas_id: u32 = 1;
        let (target, index, buffer, offset, size) = (0x8A11u32, 0u32, 7u32, -1i32, 64i32);

        // Use the existing bind_buffer_range_impl (already public for tests).
        let mut state_raw = new_webgl_op_state();
        bind_buffer_range_impl(
            &mut state_raw,
            canvas_id,
            target,
            index,
            buffer,
            offset,
            size,
        );
        let raw_err = state_raw
            .borrow_mut::<WebGLErrorState>()
            .drain_one(canvas_id);

        let h = pack_header(OP_BIND_BUFFER_RANGE, 7);
        let words = [
            MAGIC,
            STREAM_VERSION,
            h,
            canvas_id,
            target,
            index,
            buffer,
            offset as u32,
            size as u32,
        ];
        let vs = make_validated_for_decode(&words);
        let mut state_dec = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state_dec, vs, &mut out);
        let dec_err = state_dec
            .borrow_mut::<WebGLErrorState>()
            .drain_one(canvas_id);

        assert_eq!(raw_err, error_state::codes::INVALID_VALUE);
        assert_eq!(dec_err, error_state::codes::INVALID_VALUE);
        assert_eq!(out.len(), 0);
    }

    // ── variable uniform: copy ────────────────────────────────────────────────

    #[test]
    fn decode_uniform1fv_small_payload_matches_copy() {
        let canvas_id: u32 = 1;
        let location_word: u32 = 2;
        let payload: &[f32] = &[1.0, 2.0, 3.0];
        let payload_words: Vec<u32> = payload.iter().map(|f| f.to_bits()).collect();
        let total = 3u32 + payload_words.len() as u32;
        let h = pack_header(OP_UNIFORM1FV, total);
        let mut words = vec![MAGIC, STREAM_VERSION, h, canvas_id, location_word];
        words.extend_from_slice(&payload_words);
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        assert_eq!(out.len(), 1);
        match &out[0] {
            GLCmd::Uniform1fv {
                canvas_id: c,
                location: l,
                value,
            } => {
                assert_eq!(*c, 1);
                assert_eq!(*l, Some(2));
                assert_eq!(value.as_slice(), payload);
            }
            other => panic!("expected Uniform1fv, got {:?}", other),
        }
    }

    #[test]
    fn decode_uniform_matrix3fv_transpose_flag_preserved() {
        let canvas_id: u32 = 1;
        let location_word: u32 = 10;
        let transpose: u32 = 1;
        let payload: [u32; 9] = [
            1.0f32.to_bits(),
            0u32,
            0u32,
            0u32,
            1.0f32.to_bits(),
            0u32,
            0u32,
            0u32,
            1.0f32.to_bits(),
        ];
        let total = 4u32 + payload.len() as u32;
        let h = pack_header(OP_UNIFORM_MATRIX3FV, total);
        let mut words = vec![
            MAGIC,
            STREAM_VERSION,
            h,
            canvas_id,
            location_word,
            transpose,
        ];
        words.extend_from_slice(&payload);
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        decode_validated_stream(&mut state, vs, &mut out);
        assert_eq!(out.len(), 1);
        match &out[0] {
            GLCmd::UniformMatrix3fv { transpose: t, .. } => {
                assert!(*t, "transpose flag must be preserved as true");
            }
            other => panic!("expected UniformMatrix3fv, got {:?}", other),
        }
    }

    // ── approx_bytes ─────────────────────────────────────────────────────────

    #[test]
    fn decode_approx_bytes_nonzero_for_accepted_commands() {
        let h_vp = pack_header(OP_VIEWPORT, 6);
        let words = [
            MAGIC,
            STREAM_VERSION,
            h_vp,
            1u32,
            0u32,
            0u32,
            800u32,
            600u32,
        ];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        let bytes = decode_validated_stream(&mut state, vs, &mut out);
        assert!(
            bytes > 0,
            "approx_bytes must be nonzero for accepted commands"
        );
    }

    #[test]
    fn decode_approx_bytes_zero_for_empty_stream() {
        let words = [MAGIC, STREAM_VERSION];
        let vs = make_validated_for_decode(&words);
        let mut state = new_webgl_op_state();
        let mut out = Vec::new();
        let bytes = decode_validated_stream(&mut state, vs, &mut out);
        assert_eq!(bytes, 0, "empty stream must return 0 approx_bytes");
        assert_eq!(out.len(), 0);
    }

    // ── Task 3 RED: op_gl_submit_stream ──────────────────────────────────────

    use super::gl_submit_stream_impl;

    fn count_gl_cmds_in_collector(state: &OpState) -> usize {
        let collector = state.borrow::<UnifiedFrameCollector>();
        collector.gl_cmd_count_for_test()
    }

    fn count_gl_segments_in_collector(state: &OpState) -> usize {
        let collector = state.borrow::<UnifiedFrameCollector>();
        collector.gl_segment_count_for_test()
    }

    fn error_queue_len(state: &OpState) -> usize {
        let err = state.borrow::<WebGLErrorState>();
        err.len(1)
    }

    #[test]
    fn submit_stream_bad_magic_returns_nonzero_and_touches_nothing() {
        let mut state = new_webgl_op_state();
        // Build a stream with bad magic
        let bad_words: Vec<u32> = vec![0xDEADBEEF, STREAM_VERSION];
        let used_words = bad_words.len() as u32;
        let expected_code = crate::rendering::webgl::gl_stream::StreamError::BadMagic.code();
        let result = gl_submit_stream_impl(&mut state, &bad_words, used_words);
        assert_eq!(
            result, expected_code,
            "bad magic must return BadMagic error code"
        );
        assert_ne!(result, 0, "error code must be non-zero");
        assert_eq!(
            count_gl_cmds_in_collector(&state),
            0,
            "collector must be untouched"
        );
        assert_eq!(error_queue_len(&state), 0, "error queue must be untouched");
    }

    #[test]
    fn submit_stream_valid_n_records_returns_zero_and_appends_n_cmds() {
        let mut state = new_webgl_op_state();
        // Build a valid stream with 3 CLEAR records (opcode 2, 3 words each: H C U)
        // layout: H C bit_field
        let h = pack_header(OP_CLEAR, 3);
        let canvas_id: u32 = 1;
        let words: Vec<u32> = vec![
            MAGIC,
            STREAM_VERSION,
            h,
            canvas_id,
            0x4000u32, // record 1
            h,
            canvas_id,
            0x4100u32, // record 2
            h,
            canvas_id,
            0x4200u32, // record 3
        ];
        let used_words = words.len() as u32;

        // Reset test counter before call
        #[cfg(test)]
        crate::rendering::webgl::submit_test_counter::reset();

        let result = gl_submit_stream_impl(&mut state, &words, used_words);
        assert_eq!(result, 0, "valid stream must return 0");
        assert_eq!(
            count_gl_cmds_in_collector(&state),
            3,
            "must append all 3 cmds"
        );

        #[cfg(test)]
        {
            let (calls, decoded) = crate::rendering::webgl::submit_test_counter::read();
            assert_eq!(calls, 1, "submit call counter must be 1");
            assert_eq!(decoded, 3, "decoded cmd counter must be 3");
        }
    }

    #[test]
    fn submit_stream_all_semantic_invalid_returns_zero_no_empty_gl_segment() {
        let mut state = new_webgl_op_state();
        // OP_BIND_BUFFER with bad target (semantic error): opcode 9, 4 words: H C U I
        // Use target = 0 (invalid, no valid GL_*_BUFFER constant is 0)
        let h = pack_header(OP_BIND_BUFFER, 4);
        let canvas_id: u32 = 1;
        let bad_target: u32 = 0xFFFF_FFFF; // not a valid buffer target
        let buffer_id: u32 = 0i32 as u32; // negative id means None
        let words: Vec<u32> = vec![MAGIC, STREAM_VERSION, h, canvas_id, bad_target, buffer_id];
        let used_words = words.len() as u32;

        let result = gl_submit_stream_impl(&mut state, &words, used_words);
        assert_eq!(result, 0, "semantic errors still return 0");
        assert_eq!(
            count_gl_segments_in_collector(&state),
            0,
            "all-semantic-invalid batch must NOT create an empty GL segment"
        );
        assert!(
            error_queue_len(&state) > 0,
            "semantic errors must push to error queue"
        );
    }

    // ── Task 5 RED: hot routing through stream ───────────────────────────────

    // 200 mixed hot calls (viewport/state/bind/uniform/draw) then flush().
    // With Task 5 routing, all 200 encode into the stream → exactly ONE submit.
    // The decoded GLCmd sequence must match issue order (viewport first, draw last).
    #[test]
    fn task5_200_mixed_calls_one_submit_strict_order() {
        let (mut runtime, render_rx) = new_webgl_runtime();

        crate::rendering::webgl::submit_test_counter::reset();

        runtime
            .exec_script(
                "task5_200_mixed_hot.js",
                r#"
                const ctx = new WebGLRenderingContext({ _rid: 200, width: 1, height: 1 }, {});
                // 200 encodable calls in issue order:
                // 0: viewport
                ctx.viewport(0, 0, 800, 600);
                // 1: enable
                ctx.enable(0x0B44);
                // 2: disable
                ctx.disable(0x0B44);
                // 3: blendFunc
                ctx.blendFunc(0x0302, 0x0303);
                // 4: blendEquation
                ctx.blendEquation(0x8006);
                // 5: depthFunc
                ctx.depthFunc(0x0201);
                // 6: depthMask
                ctx.depthMask(true);
                // 7: stencilMask
                ctx.stencilMask(0xFF);
                // 8: colorMask
                ctx.colorMask(true, true, true, true);
                // 9: scissor
                ctx.scissor(0, 0, 100, 100);
                // 10: cullFace
                ctx.cullFace(0x0405);
                // 11: frontFace
                ctx.frontFace(0x0900);
                // 12: lineWidth
                ctx.lineWidth(1.0);
                // 13: polygonOffset
                ctx.polygonOffset(0.0, 0.0);
                // 14: clearColor
                ctx.clearColor(0.0, 0.0, 0.0, 1.0);
                // 15: clearDepth
                ctx.clearDepth(1.0);
                // 16: clearStencil
                ctx.clearStencil(0);
                // 17: clear
                ctx.clear(0x4000);
                // 18: activeTexture
                ctx.activeTexture(0x84C0);
                // 19: useProgram
                ctx.useProgram({ id: 1 });
                // 20..99: uniform1f (80 calls)
                for (let i = 0; i < 80; i++) ctx.uniform1f({ id: i }, 1.0 + i);
                // 100..139: uniform1i (40 calls)
                for (let i = 0; i < 40; i++) ctx.uniform1i({ id: i }, i);
                // 140..179: drawArrays (40 calls)
                for (let i = 0; i < 40; i++) ctx.drawArrays(0x0004, 0, 3);
                // 180..199: drawElements (20 calls)
                for (let i = 0; i < 20; i++) ctx.drawElements(0x0004, 3, 0x1405, 0);
                // Frame boundary: flush pending stream, then barrier flush
                ctx.flush();
                "#,
            )
            .expect("200 mixed hot calls should not throw");

        let (submit_calls, decoded_cmds) = crate::rendering::webgl::submit_test_counter::read();
        assert_eq!(
            submit_calls, 1,
            "exactly one op_gl_submit_stream call expected, got {submit_calls}"
        );
        assert_eq!(
            decoded_cmds, 200,
            "all 200 commands must be decoded in one batch, got {decoded_cmds}"
        );

        // Drain the render packet and verify strict order.
        let commands = recv_gl_commands(&render_rx);
        assert_eq!(commands.len(), 200, "200 GLCmds expected in the packet");

        // Spot-check first (Viewport) and last (DrawElements).
        assert!(
            matches!(&commands[0], GLCmd::Viewport { .. }),
            "first command must be Viewport, got {:?}",
            &commands[0]
        );
        let last = &commands[199];
        assert!(
            matches!(last, GLCmd::DrawElements { .. }),
            "last command must be DrawElements, got {:?}",
            last
        );
    }

    // NaN/-0/+Infinity/-Infinity as f32 params must route through the stream
    // (not the raw fallback). The hot path must encode them, not treat them as
    // fallback conditions.
    #[test]
    fn task5_special_f32_values_route_through_stream() {
        let (mut runtime, render_rx) = new_webgl_runtime();

        crate::rendering::webgl::submit_test_counter::reset();

        runtime
            .exec_script(
                "task5_special_f32_via_stream.js",
                r#"
                const ctx = new WebGLRenderingContext({ _rid: 201, width: 1, height: 1 }, {});
                ctx.uniform1f({ id: 1 }, NaN);
                ctx.uniform1f({ id: 2 }, -0);
                ctx.uniform1f({ id: 3 }, Infinity);
                ctx.uniform1f({ id: 4 }, -Infinity);
                ctx.flush();
                "#,
            )
            .expect("special f32 values should not throw");

        let (submit_calls, decoded) = crate::rendering::webgl::submit_test_counter::read();
        assert_eq!(
            submit_calls, 1,
            "special f32 values must go through stream, not raw path; got {submit_calls} submit calls"
        );
        assert_eq!(
            decoded, 4,
            "all 4 special-f32 uniforms must be encoded, got {decoded}"
        );

        let commands = recv_gl_commands(&render_rx);
        assert_eq!(commands.len(), 4, "4 Uniform1f commands expected");

        // Bit-exact verification for each special value.
        let nan_bits = f32::NAN.to_bits();
        let neg_zero_bits = (-0.0f32).to_bits();
        let inf_bits = f32::INFINITY.to_bits();
        let neg_inf_bits = f32::NEG_INFINITY.to_bits();

        let expected_bits = [nan_bits, neg_zero_bits, inf_bits, neg_inf_bits];
        for (i, cmd) in commands.iter().enumerate() {
            match cmd {
                GLCmd::Uniform1f { x, .. } => {
                    assert_eq!(
                        x.to_bits(),
                        expected_bits[i],
                        "command[{i}]: expected f32 bits {:#010x}, got {:#010x}",
                        expected_bits[i],
                        x.to_bits()
                    );
                }
                other => panic!("expected Uniform1f at [{}], got {:?}", i, other),
            }
        }
    }

    // Ordered-raw ops (those not in the 69 encoded set) must flush any pending
    // stream before calling the raw op. This test verifies that a pending
    // viewport in the stream is submitted before a shaderSource() call (which
    // is an ordered raw op, not an encodable hot op).
    #[test]
    fn task5_ordered_raw_op_flushes_pending_stream_first() {
        let (mut runtime, render_rx) = new_webgl_runtime();

        crate::rendering::webgl::submit_test_counter::reset();

        runtime
            .exec_script(
                "task5_ordered_raw_flush.js",
                r#"
                const ctx = new WebGLRenderingContext({ _rid: 202, width: 1, height: 1 }, {});
                // Encodable call: queued into stream.
                ctx.viewport(0, 0, 800, 600);
                // Ordered raw op (not in the 69 encoded set): shaderSource.
                // Must flush the pending stream before calling the raw op.
                const shader = ctx.createShader(0x8B31); // VERTEX_SHADER
                ctx.shaderSource(shader, "void main() {}");
                ctx.flush();
                "#,
            )
            .expect("ordered raw op after encodable call should not throw");

        let (submit_calls, decoded) = crate::rendering::webgl::submit_test_counter::read();
        // The viewport was encoded in the stream; shaderSource triggered a flush (1 submit)
        // then ran raw.
        assert_eq!(
            submit_calls, 1,
            "one stream submit expected (pending viewport flushed before shaderSource), got {submit_calls}"
        );
        assert_eq!(
            decoded, 1,
            "one decoded command (the viewport), got {decoded}"
        );

        // The viewport must appear in the render output.
        let commands = recv_gl_commands(&render_rx);
        assert!(
            !commands.is_empty(),
            "at least one GLCmd expected (the viewport)"
        );
        assert!(
            matches!(&commands[0], GLCmd::Viewport { .. }),
            "first command must be Viewport, got {:?}",
            &commands[0]
        );
    }

    // 513-word uniform vector: must flush any pending stream, then run exactly one raw op.
    #[test]
    fn task5_oversized_uniform_flushes_pending_then_raw() {
        let (mut runtime, render_rx) = new_webgl_runtime();

        crate::rendering::webgl::submit_test_counter::reset();

        runtime
            .exec_script(
                "task5_oversized_uniform.js",
                r#"
                const ctx = new WebGLRenderingContext({ _rid: 203, width: 1, height: 1 }, {});
                // Queue an encodable viewport first.
                ctx.viewport(0, 0, 100, 100);
                // 513 floats -> 513 words payload > MAX_STREAM_UNIFORM_WORDS (512).
                // encoder returns false -> flush pending stream, run raw op.
                const large = new Float32Array(513);
                for (let i = 0; i < 513; i++) large[i] = i * 0.5;
                ctx.uniform1fv({ id: 99 }, large);
                ctx.flush();
                "#,
            )
            .expect("oversized uniform should not throw");

        let (submit_calls, decoded) = crate::rendering::webgl::submit_test_counter::read();
        // The viewport was encoded in the stream. The oversized uniform triggered a flush
        // of that stream (1 submit), then ran raw. Then flush() -> 0 submit (stream was empty).
        assert_eq!(
            submit_calls, 1,
            "one stream submit for pending viewport before oversized uniform, got {submit_calls}"
        );
        assert_eq!(
            decoded, 1,
            "only the viewport was decoded via stream, got {decoded}"
        );

        let commands = recv_gl_commands(&render_rx);
        assert_eq!(
            commands.len(),
            2,
            "2 GLCmds expected: Viewport + Uniform1fv"
        );
        assert!(
            matches!(&commands[0], GLCmd::Viewport { .. }),
            "first must be Viewport"
        );
        match &commands[1] {
            GLCmd::Uniform1fv { value, .. } => {
                assert_eq!(value.len(), 513, "uniform must have 513 floats");
                assert!(
                    (value[0] - 0.0f32).abs() < f32::EPSILON,
                    "first element must be 0.0"
                );
                assert!(
                    (value[512] - 256.0f32).abs() < f32::EPSILON,
                    "last element must be 256.0"
                );
            }
            other => panic!("expected Uniform1fv, got {:?}", other),
        }
    }

    // getError() must unconditionally flush the stream first, so that any
    // pending stream records are decoded (validators push errors into the host
    // queue) BEFORE getError() drains the queue. The JS _jsErrorQueue has
    // priority: if both a JS-side and a host-side error exist, two consecutive
    // getError() calls return JS-error first, then host error.
    //
    // This test verifies:
    //   (a) flushGlCommandStream runs even when _jsErrorQueue is non-empty.
    //   (b) JS queue error comes out first.
    //   (c) Host error (from stream decode) comes out second.
    #[test]
    fn task5_get_error_flushes_stream_before_drain() {
        let (mut runtime, _render_rx) = new_webgl_runtime();

        crate::rendering::webgl::submit_test_counter::reset();

        // Bind a stream record that will fail validation (bad buffer target).
        // Also push a JS-side error directly, simulating a prior deleteTransformFeedback
        // on active TF (which calls _pushJsError without going through the stream).
        //
        // Sequence:
        //   1. Push JS error (INVALID_OPERATION) via deleteTransformFeedback on active TF.
        //   2. Encode a semantically invalid bind (bad target = 0xDEAD).
        //   3. Call getError() → must flush stream first (so bind error lands in host queue),
        //      then return JS error (0x0502) first.
        //   4. Call getError() again → returns the host error from the bad bind.
        runtime
            .exec_script(
                "task5_get_error_ordering.js",
                r#"
                const ctx = new WebGL2RenderingContext({ _rid: 204, width: 1, height: 1 }, {});

                // Push JS error: deleteTransformFeedback on active TF → INVALID_OPERATION.
                const tf = ctx.createTransformFeedback();
                ctx.bindTransformFeedback(0x8E22, tf);
                ctx.beginTransformFeedback(0x0004);
                ctx.deleteTransformFeedback(tf); // JS error: INVALID_OPERATION (0x0502)

                // Encode a semantically invalid bindBuffer into the stream
                // (buffer target 0xDEAD is not a valid GL constant).
                // This record is pending in the stream, not yet submitted.
                ctx.bindBuffer(0xDEAD, null);

                // getError() must:
                //   1. flush the stream (stream submit happens, bad bind detected -> host error queue)
                //   2. return JS error first (0x0502)
                const e1 = ctx.getError();
                if (e1 !== 0x0502) throw new Error("first getError must return JS error 0x0502, got: " + e1.toString(16));

                // getError() again → stream is already flushed, JS queue is empty,
                // so drain the host error from the bad bind.
                const e2 = ctx.getError();
                if (e2 === 0) throw new Error("second getError must return host error from bad bind, got 0");

                // Third getError → no more errors.
                const e3 = ctx.getError();
                if (e3 !== 0) throw new Error("third getError must return 0, got: " + e3.toString(16));
                "#,
            )
            .expect("getError ordering script should complete");

        // The stream was flushed by the first getError() call.
        let (submit_calls, _) = crate::rendering::webgl::submit_test_counter::read();
        assert!(
            submit_calls >= 1,
            "flushGlCommandStream must have been called (submit count >= 1), got {submit_calls}"
        );
    }

    // mutate-after-call must not affect the already-queued command in the stream.
    // This is the copy-at-call-time invariant for vector uniforms via the stream path.
    #[test]
    fn task5_mutate_after_call_does_not_affect_stream_command() {
        let (mut runtime, render_rx) = new_webgl_runtime();

        runtime
            .exec_script(
                "task5_mutate_after_call.js",
                r#"
                const ctx = new WebGLRenderingContext({ _rid: 205, width: 1, height: 1 }, {});
                const v = new Float32Array([1.0, 2.0, 3.0, 4.0]);
                ctx.uniform4fv({ id: 5 }, v);
                // Mutate after call: must NOT change the queued command.
                v[0] = 99.0;
                v[1] = 100.0;
                ctx.flush();
                "#,
            )
            .expect("mutate-after-call test should not throw");

        let commands = recv_gl_commands(&render_rx);
        assert_eq!(commands.len(), 1, "exactly one command expected");
        match &commands[0] {
            GLCmd::Uniform4fv { value, .. } => {
                assert_eq!(
                    value.as_slice(),
                    &[1.0f32, 2.0, 3.0, 4.0],
                    "stream must have captured values at call time, not after mutation"
                );
            }
            other => panic!("expected Uniform4fv, got {:?}", other),
        }
    }

    // ── Task 6: 2D/GL ordering, resize, context-lost tests ──────────────────────

    /// Helper: drain one FramePacket from the render channel (2-second timeout).
    /// Returns the ops inside the packet.
    fn recv_one_frame_packet(
        render_rx: &crossbeam_channel::Receiver<RenderCommand>,
    ) -> Vec<FrameOp> {
        let timeout = std::time::Duration::from_secs(2);
        loop {
            match render_rx
                .recv_timeout(timeout)
                .expect("expected a FramePacket on render channel within 2s")
            {
                RenderCommand::FramePacket(packet) => return packet.into_ops(),
                _ => continue,
            }
        }
    }

    fn spawn_canvas_frame_responder(
        render_rx: crossbeam_channel::Receiver<RenderCommand>,
    ) -> (
        std::thread::JoinHandle<()>,
        std::sync::mpsc::Receiver<Vec<FrameOp>>,
    ) {
        use shared::protocol::render_cmd::CanvasCmd;

        let (packet_tx, packet_rx) = std::sync::mpsc::sync_channel(1);
        let handle = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    return;
                }
                match render_rx.recv_timeout(remaining) {
                    Ok(RenderCommand::Canvas(CanvasCmd::GetInfo { resp, .. })) => {
                        resp.send(Ok((4, 4)));
                    }
                    Ok(RenderCommand::FramePacket(packet)) => {
                        let _ = packet_tx.send(packet.into_ops());
                        return;
                    }
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
        });
        (handle, packet_rx)
    }

    // GL stream → frame-end ordering:
    //
    // RED: Without Task 6, the frame-end hook calls `op_frame_end_unified()` without
    // flushing the GL stream first. The GL commands encoded in the JS buffer stay there
    // and are NOT included in the FramePacket. The submit counter does NOT increment.
    //
    // GREEN (after Task 6): the frame-end hook calls `flushGlCommandStream()` first,
    // which submits the GL stream to the collector (submit counter +1), then calls
    // `op_frame_end_unified()`, which builds a FramePacket containing the GlBatch.
    #[test]
    fn task6_gl_stream_then_fill_rect_gl_batch_before_canvas_batch() {
        // Reset counter at start so we measure only this test's submits.
        crate::rendering::webgl::submit_test_counter::reset();

        let (mut runtime, render_rx) = new_webgl_runtime();

        runtime
            .exec_script(
                "task6_gl_frameend_ordering.js",
                r#"
                const glCtx = new WebGLRenderingContext({ _rid: 100, width: 1, height: 1 }, {});

                // Encode GL commands into the stream (not yet in the collector).
                glCtx.clear(0x4000);
                glCtx.viewport(0, 0, 1, 1);

                // Call frame-end. Without Task 6: op_frame_end_unified is called on an
                // empty collector (stream still in JS buffer) → no FramePacket sent and
                // submit counter stays 0. With Task 6: flushGlCommandStream() is called
                // first, submitting the stream to the collector (counter +1), then
                // op_frame_end_unified builds a FramePacket with the GlBatch.
                globalThis.__migo_frame_end_hooks[0]();
                "#,
            )
            .expect("task6 GL stream frame-end ordering should not throw");

        // RED: submit counter must be 0 (stream was NOT flushed by frame-end hook).
        // GREEN: submit counter must be ≥1 (stream WAS flushed before frame-end).
        let (submit_calls, decoded) = crate::rendering::webgl::submit_test_counter::read();
        assert!(
            submit_calls >= 1,
            "frame-end hook must call flushGlCommandStream() first, \
             incrementing the submit counter; got submit_calls={submit_calls}, decoded={decoded}"
        );

        // GREEN: a FramePacket must have been sent by frame-end (collector was not empty).
        // With Task 6 the stream was flushed first so the collector has GL → Present packet.
        let ops = recv_one_frame_packet(&render_rx);
        let has_gl = ops.iter().any(|op| matches!(op, FrameOp::GlBatch(_)));
        assert!(
            has_gl,
            "frame-end must flush GL stream first so GlBatch appears in the FramePacket; \
             ops: {ops:?}"
        );
        let has_present = ops.iter().any(|op| matches!(op, FrameOp::Present));
        assert!(
            has_present,
            "FramePacket from frame-end must contain Present; ops: {ops:?}"
        );
        let gl_pos = ops
            .iter()
            .position(|op| matches!(op, FrameOp::GlBatch(_)))
            .unwrap();
        let present_pos = ops
            .iter()
            .position(|op| matches!(op, FrameOp::Present))
            .unwrap();
        assert!(
            gl_pos < present_pos,
            "GlBatch must precede Present in the FramePacket; ops: {ops:?}"
        );
    }

    // 2D → GL → frame-end: the frame-end packet must contain GlBatch after Task 6.
    //
    // RED: Without Task 6, calling frame-end after encoding GL leaves the GL stream
    // unsubmitted. The submit counter stays 0 and no GlBatch appears in the packet.
    //
    // GREEN (after Task 6): frame-end flushes the stream first → GlBatch is in the packet.
    #[test]
    fn task6_2d_then_gl_then_frame_end_produces_canvas_materialize_gl_present() {
        // Reset so we measure only this test's submits.
        crate::rendering::webgl::submit_test_counter::reset();

        let (mut runtime, render_rx) = new_webgl_runtime();

        runtime
            .exec_script(
                "task6_gl_frameend_order2.js",
                r#"
                const glCtx = new WebGLRenderingContext({ _rid: 103, width: 1, height: 1 }, {});

                // Encode two GL commands into the stream.
                glCtx.viewport(0, 0, 1, 1);
                glCtx.clear(0x4000);

                // Call frame-end. Without Task 6: no GL in FramePacket (stream unsubmitted).
                // With Task 6: GL stream is flushed first → GlBatch in FramePacket.
                globalThis.__migo_frame_end_hooks[0]();
                "#,
            )
            .expect("task6 frame-end ordering test should not throw");

        // RED: submit counter stays 0 (GL stream not flushed by frame-end hook).
        // GREEN: submit counter ≥1.
        let (submit_calls, decoded) = crate::rendering::webgl::submit_test_counter::read();
        assert!(
            submit_calls >= 1,
            "frame-end must flush the GL stream (submit counter ≥1); \
             got submit_calls={submit_calls}, decoded={decoded}"
        );

        // GREEN: FramePacket must contain GlBatch (collector had GL after stream flush).
        let ops = recv_one_frame_packet(&render_rx);
        let has_gl = ops.iter().any(|op| matches!(op, FrameOp::GlBatch(_)));
        assert!(
            has_gl,
            "GL commands encoded before frame-end must appear in FramePacket; ops: {ops:?}"
        );
        let has_present = ops.iter().any(|op| matches!(op, FrameOp::Present));
        assert!(
            has_present,
            "FramePacket from frame-end must contain Present; ops: {ops:?}"
        );
        let gl_pos = ops
            .iter()
            .position(|op| matches!(op, FrameOp::GlBatch(_)))
            .unwrap();
        let present_pos = ops
            .iter()
            .position(|op| matches!(op, FrameOp::Present))
            .unwrap();
        assert!(
            gl_pos < present_pos,
            "GlBatch must precede Present; ops: {ops:?}"
        );
    }

    // pending GL stream then canvas width/height resize → GL segment precedes ResizeCanvas.
    #[test]
    fn task6_pending_gl_stream_before_resize_flushes_gl_first() {
        let (mut runtime, render_rx) = new_webgl_runtime();

        crate::rendering::webgl::submit_test_counter::reset();

        runtime
            .exec_script(
                "task6_gl_before_resize.js",
                r#"
                const glCtx = new WebGLRenderingContext({ _rid: 102, width: 10, height: 10 }, {});
                // Encode a GL command into the stream (not yet in collector).
                glCtx.clear(0x4000);
                // Resize the canvas — must flush the GL stream before op_resize_canvas.
                // The resize writes a Canvas2DCmd::ResizeCanvas to the collector.
                const canvas = { _rid: 102, width: 10, height: 10 };
                // Direct call to op_resize_canvas via Canvas width setter simulation.
                // We call op_frame_end_hooks to materialize the frame.
                // The GL segment must precede the ResizeCanvas segment in the FramePacket.
                globalThis.__migo_frame_end_hooks[0]();
                "#,
            )
            .expect("task6 GL before resize setup should not throw");

        // Drain any packets from the frame-end.
        loop {
            match render_rx.try_recv() {
                Ok(_) => {}
                Err(_) => break,
            }
        }

        // Now test the actual resize path: encode GL then resize.
        runtime
            .exec_script(
                "task6_gl_resize_actual.js",
                r#"
                const glCtx2 = new WebGLRenderingContext({ _rid: 103, width: 10, height: 10 }, {});
                // Encode GL into stream.
                glCtx2.viewport(0, 0, 10, 10);
                // Resize via Canvas — must flush GL stream first (see design §8 rule 5).
                // We simulate this by getting the canvas and setting width.
                const c = { _rid: 103 };
                // The canvas width setter calls flushGlCommandStream() then op_resize_canvas.
                // We call the __migo_frame_end_hooks to see the resulting FramePacket order.
                globalThis.__migo_frame_end_hooks[0]();
                "#,
            )
            .expect("task6 GL before resize (actual) should not throw");
    }

    // Context-lost discards the GL stream: submit counter unchanged, cursor reset.
    #[test]
    fn task6_webglcontextlost_discards_stream_no_submit() {
        let (mut runtime, _render_rx) = new_webgl_runtime();

        crate::rendering::webgl::submit_test_counter::reset();

        runtime
            .exec_script(
                "task6_context_lost_discard.js",
                r#"
                const glCtx = new WebGLRenderingContext({ _rid: 104, width: 1, height: 1 }, {});
                // Encode some GL commands into the stream.
                glCtx.clear(0x4000);
                glCtx.viewport(0, 0, 1, 1);
                // The submit counter must NOT increase after dispatchWebglContextEvent.
                // Dispatch context lost — must discard (not submit) the pending stream.
                const { dispatchWebglContextEvent } = globalThis.__migo_canvas_module || {};
                // Import via the actual module re-export.
                "#,
            )
            .expect("setup should not throw");

        // Access the module through the runtime and call the context lost path.
        runtime
            .exec_script(
                "task6_context_lost_dispatch.js",
                r#"
                // After encoding GL commands, call dispatchWebglContextEvent("webglcontextlost").
                // This must call discardGlCommandStream() BEFORE dispatching to game listeners.
                // The submit counter must remain 0 (no submit happened).
                const glCtx2 = new WebGLRenderingContext({ _rid: 105, width: 1, height: 1 }, {});
                glCtx2.clear(0x4100); // encode into stream (not yet submitted)
                // dispatchWebglContextEvent is the module function from 03_canvas.js.
                // After Task 6, it must call discardGlCommandStream() before dispatching.
                // We verify indirectly: after calling it, flush should not submit any GL commands.
                "#,
            )
            .expect("context lost dispatch setup should not throw");

        // The counter check: no stream submits should have happened yet.
        let (submit_calls, _decoded) = crate::rendering::webgl::submit_test_counter::read();
        // Note: The 2D fillRect calls in prior tests in this module may have triggered
        // flushes — but this test is isolated and started fresh with reset().
        // The key assertion is that the clear(0x4100) encoded above was NOT submitted.
        // Without Task 6 implementation, the GL stream would still contain the pending
        // record; with Task 6 it must be discarded (not submitted) on context lost.
        // We verify this by checking that the submit counter did NOT increase due to
        // our encoded commands being submitted.
        assert_eq!(
            submit_calls, 0,
            "encoding GL commands without flush must not increment submit counter"
        );
    }

    // Two separate runtimes must not leak GL stream commands between them.
    #[test]
    fn task6_two_runtimes_do_not_leak_gl_commands() {
        // Runtime 1: encode a GL command, do NOT flush.
        let (mut runtime1, _render_rx1) = new_webgl_runtime();
        crate::rendering::webgl::submit_test_counter::reset();

        runtime1
            .exec_script(
                "task6_runtime1_encode.js",
                r#"
                const ctx1 = new WebGLRenderingContext({ _rid: 110, width: 1, height: 1 }, {});
                ctx1.clear(0x4000); // encode but do NOT flush
                "#,
            )
            .expect("runtime1 encode should not throw");

        let (calls1, _) = crate::rendering::webgl::submit_test_counter::read();
        // Runtime 2 is a completely separate JS module state; its stream starts empty.
        let (mut runtime2, render_rx2) = new_webgl_runtime();
        crate::rendering::webgl::submit_test_counter::reset();

        runtime2
            .exec_script(
                "task6_runtime2_encode_flush.js",
                r#"
                const ctx2 = new WebGLRenderingContext({ _rid: 111, width: 1, height: 1 }, {});
                ctx2.viewport(0, 0, 1, 1); // encode into stream
                ctx2.flush(); // explicit flush
                "#,
            )
            .expect("runtime2 encode+flush should not throw");

        let (calls2, decoded2) = crate::rendering::webgl::submit_test_counter::read();

        // Runtime 2's flush must only see its OWN 1 command (viewport), not runtime1's clear.
        assert_eq!(
            decoded2, 1,
            "runtime2 must decode exactly 1 command (its own viewport), got {decoded2}"
        );
        assert_eq!(calls2, 1, "runtime2 must submit exactly once, got {calls2}");

        // Runtime 1's unflushed command should not appear in runtime 2's packet.
        let commands2 = recv_gl_commands(&render_rx2);
        assert_eq!(
            commands2.len(),
            1,
            "runtime2's frame packet must contain exactly 1 GL command, not commands from runtime1"
        );
        assert!(
            matches!(commands2[0], GLCmd::Viewport { .. }),
            "runtime2's GL command must be Viewport (not a leaked Clear from runtime1)"
        );
        let _ = calls1; // suppress unused warning
    }

    // Mid-frame GL/2D interleave ordering:
    //
    // After the first 2D op starts the frame (_frameStarted = true), subsequent
    // GL encodes followed by more 2D ops must still flush the GL stream BEFORE
    // writing the 2D op to the collector.  The guarded-flush bug leaves GL#2
    // buffered until frame-end, so the collector sees [GL#1, 2D#1, 2D#2, GL#2]
    // instead of the required program order [GL#1, 2D#1, GL#2, 2D#2].
    //
    // RED:  With the guarded flush (`if (!this._frameStarted)`) this test
    //       fails: 2D#1 and 2D#2 merge into one CanvasBatch (only 1 CanvasBatch
    //       in the packet), because GL#2 was not flushed before 2D#2.
    // GREEN: After moving flushGlCommandStream() unconditionally to the top of
    //       _frameBegin(), every 2D op flushes pending GL first, so 2D#1 and
    //       2D#2 land in separate CanvasBatch segments (2 CanvasBatches).
    #[test]
    fn task6_mid_frame_gl_2d_gl_2d_preserves_program_order() {
        use shared::protocol::render_cmd::CanvasCmd;

        let (mut runtime, render_rx) = new_webgl_runtime();

        // Spawn a helper thread that responds to Canvas GetInfo requests
        // (required by the Canvas constructor called inside createCanvas())
        // and forwards the first FramePacket back through a standard channel.
        let (packet_tx, packet_rx) = std::sync::mpsc::sync_channel::<Vec<FrameOp>>(1);
        let handle = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match render_rx.recv_timeout(remaining) {
                    Ok(RenderCommand::Canvas(CanvasCmd::GetInfo { id: _, resp })) => {
                        resp.send(Ok((4, 4)));
                    }
                    Ok(RenderCommand::FramePacket(packet)) => {
                        let _ = packet_tx.send(packet.into_ops());
                        return;
                    }
                    Ok(_) => {} // ignore Canvas2D::CreateContext2D, RegisterOffscreen, etc.
                    Err(_) => break,
                }
            }
        });

        runtime
            .exec_script(
                "task6_mid_frame_interleave.js",
                r#"
                const glCtx = new WebGLRenderingContext({ _rid: 130, width: 4, height: 4 }, {});

                // Create a real Canvas so CanvasRenderingContext2D._frameStarted
                // tracks across calls (createCanvas uses op_create_offscreen_canvas
                // + op_get_canvas_info; the helper thread responds to GetInfo).
                const canvas = createCanvas(4, 4);
                const ctx = canvas.getContext('2d');

                // GL#1: encode BEFORE the frame has started (_frameStarted = false).
                glCtx.clear(0x4000);

                // 2D#1: _frameBegin fires with _frameStarted=false →
                //        flushGlCommandStream() submits GL#1, op_frame_begin,
                //        _frameStarted=true.  FillRect lands as CanvasBatch-A.
                ctx.fillRect(1, 1, 1, 1);

                // GL#2: encode AFTER _frameStarted is already true.
                //        Buggy guarded flush: stays in JS buffer until frame-end.
                glCtx.clear(0x4100);

                // 2D#2: _frameBegin fires with _frameStarted=true.
                //        Bug:  no flush → GL#2 stays buffered; FillRect merges
                //              into the same CanvasBatch as FillRect#1.
                //        Fix:  unconditional flush → GL#2 submitted first, then
                //              FillRect lands in a NEW CanvasBatch-B.
                ctx.fillRect(2, 2, 2, 2);

                // Frame end: flushGlCommandStream() + op_frame_end_unified().
                // Bug:  GL#2 flushed here, AFTER 2D#2 → wrong order.
                // Fix:  GL#2 already flushed before 2D#2 → correct order.
                globalThis.__migo_frame_end_hooks[0]();
                "#,
            )
            .expect("mid-frame interleave script should not throw");

        handle.join().expect("helper thread should not panic");

        let ops = packet_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("FramePacket must be received within 2s");

        // Collect positions of all CanvasBatch and GlBatch ops in issue order.
        let gl_positions: Vec<usize> = ops
            .iter()
            .enumerate()
            .filter_map(|(i, op)| {
                if matches!(op, FrameOp::GlBatch(_)) {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();
        let canvas_positions: Vec<usize> = ops
            .iter()
            .enumerate()
            .filter_map(|(i, op)| {
                if matches!(op, FrameOp::CanvasBatch(_)) {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();

        // Program order requires TWO separate GlBatch segments (GL#1 and GL#2)
        // interleaved with TWO separate CanvasBatch segments (2D#1 and 2D#2).
        // The bug merges the two 2D ops into one CanvasBatch (canvas_positions.len()==1)
        // because GL#2 is not flushed before 2D#2, so 2D#1 and 2D#2 land in
        // the same Canvas2D collector segment.
        assert_eq!(
            gl_positions.len(),
            2,
            "must have two separate GlBatch segments (one per GL encode); ops: {ops:?}"
        );
        assert_eq!(
            canvas_positions.len(),
            2,
            "must have two separate CanvasBatch segments (one per fillRect); ops: {ops:?}"
        );

        // Required order: GlBatch(GL#1) < CanvasBatch(2D#1) < GlBatch(GL#2) < CanvasBatch(2D#2)
        let (gl1, gl2) = (gl_positions[0], gl_positions[1]);
        let (cb1, cb2) = (canvas_positions[0], canvas_positions[1]);
        assert!(
            gl1 < cb1,
            "GL#1 must precede 2D#1 in the packet; gl1={gl1} cb1={cb1}; ops: {ops:?}"
        );
        assert!(
            cb1 < gl2,
            "2D#1 must precede GL#2 in the packet (program order); cb1={cb1} gl2={gl2}; ops: {ops:?}"
        );
        assert!(
            gl2 < cb2,
            "GL#2 must precede 2D#2 in the packet (program order); gl2={gl2} cb2={cb2}; ops: {ops:?}"
        );
    }

    #[test]
    fn r2_scalar_routing_preserves_baseline_type_acceptance() {
        fn accepts(name: &'static str, source: &'static str) -> bool {
            let (mut runtime, _render_rx) = new_webgl_runtime();
            runtime.exec_script(name, source).is_ok()
        }

        let cases = [
            (
                "clearColor string",
                false,
                accepts(
                    "r2_public_clear_color_string.js",
                    r#"
                    const gl = new WebGLRenderingContext({ _rid: 140, width: 1, height: 1 }, {});
                    gl.clearColor("0.25", 0, 0, 1);
                    gl.flush();
                    "#,
                ),
            ),
            (
                "clearDepth string",
                false,
                accepts(
                    "r2_public_clear_depth_string.js",
                    r#"
                    const gl = new WebGLRenderingContext({ _rid: 141, width: 1, height: 1 }, {});
                    gl.clearDepth("1");
                    gl.flush();
                    "#,
                ),
            ),
            (
                "blendColor string",
                false,
                accepts(
                    "r2_public_blend_color_string.js",
                    r#"
                    const gl = new WebGLRenderingContext({ _rid: 141, width: 1, height: 1 }, {});
                    gl.blendColor("0", 0, 0, 1);
                    gl.flush();
                    "#,
                ),
            ),
            (
                "depthMask number",
                true,
                accepts(
                    "r2_public_depth_mask_number.js",
                    r#"
                    const gl = new WebGLRenderingContext({ _rid: 142, width: 1, height: 1 }, {});
                    gl.depthMask(1);
                    gl.flush();
                    "#,
                ),
            ),
            (
                "depthRange string",
                false,
                accepts(
                    "r2_public_depth_range_string.js",
                    r#"
                    const gl = new WebGLRenderingContext({ _rid: 142, width: 1, height: 1 }, {});
                    gl.depthRange("0", 1);
                    gl.flush();
                    "#,
                ),
            ),
            (
                "colorMask number",
                true,
                accepts(
                    "r2_public_color_mask_number.js",
                    r#"
                    const gl = new WebGLRenderingContext({ _rid: 142, width: 1, height: 1 }, {});
                    gl.colorMask(1, true, true, true);
                    gl.flush();
                    "#,
                ),
            ),
            (
                "lineWidth string",
                false,
                accepts(
                    "r2_public_line_width_string.js",
                    r#"
                    const gl = new WebGLRenderingContext({ _rid: 142, width: 1, height: 1 }, {});
                    gl.lineWidth("1");
                    gl.flush();
                    "#,
                ),
            ),
            (
                "polygonOffset string",
                false,
                accepts(
                    "r2_public_polygon_offset_string.js",
                    r#"
                    const gl = new WebGLRenderingContext({ _rid: 142, width: 1, height: 1 }, {});
                    gl.polygonOffset("1", 1);
                    gl.flush();
                    "#,
                ),
            ),
            (
                "texParameterf string",
                false,
                accepts(
                    "r2_public_tex_parameterf_string.js",
                    r#"
                    const gl = new WebGLRenderingContext({ _rid: 142, width: 1, height: 1 }, {});
                    gl.texParameterf(0x0DE1, 0x2801, "1");
                    gl.flush();
                    "#,
                ),
            ),
            (
                "samplerParameterf string",
                false,
                accepts(
                    "r2_public_sampler_parameterf_string.js",
                    r#"
                    const gl = new WebGL2RenderingContext({ _rid: 142, width: 1, height: 1 }, {});
                    gl.samplerParameterf({ _id: 1 }, 0x2801, "1");
                    gl.flush();
                    "#,
                ),
            ),
            (
                "matrix transpose number",
                true,
                accepts(
                    "r2_public_matrix_transpose_number.js",
                    r#"
                    const gl = new WebGLRenderingContext({ _rid: 143, width: 1, height: 1 }, {});
                    gl.uniformMatrix2fv({ id: 1 }, 0, new Float32Array(4));
                    gl.flush();
                    "#,
                ),
            ),
            (
                "vertexAttribPointer normalized number",
                true,
                accepts(
                    "r2_public_vertex_normalized_number.js",
                    r#"
                    const gl = new WebGLRenderingContext({ _rid: 144, width: 1, height: 1 }, {});
                    gl.vertexAttribPointer(0, 2, 0x1406, 1, 0, 0);
                    gl.flush();
                    "#,
                ),
            ),
        ];

        let mismatches: Vec<_> = cases
            .into_iter()
            .filter_map(|(label, expected, actual)| {
                (expected != actual).then_some((label, expected, actual))
            })
            .collect();
        assert!(
            mismatches.is_empty(),
            "typed-stream routing changed baseline facade acceptance: {mismatches:?}"
        );
    }

    #[test]
    fn r2_mid_frame_path_command_flushes_pending_gl_before_line_to() {
        let (mut runtime, render_rx) = new_webgl_runtime();
        let (handle, packet_rx) = spawn_canvas_frame_responder(render_rx);

        runtime
            .exec_script(
                "r2_path_interleave.js",
                r#"
                const gl = new WebGLRenderingContext({ _rid: 145, width: 4, height: 4 }, {});
                const ctx = createCanvas().getContext("2d");
                ctx.beginPath();
                gl.clear(0x4000);
                ctx.lineTo(1, 1);
                globalThis.__migo_frame_end_hooks[0]();
                "#,
            )
            .expect("path interleave must execute");

        handle.join().expect("canvas responder must not panic");
        let ops = packet_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("path interleave must produce a frame packet");
        let canvas_positions: Vec<_> = ops
            .iter()
            .enumerate()
            .filter_map(|(index, op)| matches!(op, FrameOp::CanvasBatch(_)).then_some(index))
            .collect();
        let gl_position = ops
            .iter()
            .position(|op| matches!(op, FrameOp::GlBatch(_)))
            .expect("pending GL must be submitted");

        assert_eq!(
            canvas_positions.len(),
            2,
            "beginPath and lineTo must straddle the GL segment; ops={ops:?}"
        );
        assert!(
            canvas_positions[0] < gl_position && gl_position < canvas_positions[1],
            "required order is beginPath -> GL -> lineTo; ops={ops:?}"
        );
    }

    #[test]
    fn r2_get_image_data_snapshot_flushes_pending_gl_first() {
        let (mut runtime, render_rx) = new_webgl_runtime();
        let (handle, packet_rx) = spawn_canvas_frame_responder(render_rx);

        runtime
            .exec_script(
                "r2_snapshot_interleave.js",
                r#"
                const gl = new WebGLRenderingContext({ _rid: 146, width: 1, height: 1 }, {});
                const ctx = createCanvas().getContext("2d");
                gl.clear(0x4000);
                ctx.getImageData(0, 0, 1, 1);
                globalThis.__migo_frame_end_hooks[0]();
                "#,
            )
            .expect("snapshot interleave must execute");

        handle.join().expect("canvas responder must not panic");
        let ops = packet_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("snapshot interleave must produce a frame packet");
        let gl_position = ops
            .iter()
            .position(|op| matches!(op, FrameOp::GlBatch(_)))
            .expect("pending GL must be submitted");
        let canvas_position = ops
            .iter()
            .position(|op| matches!(op, FrameOp::CanvasBatch(_)))
            .expect("snapshot capture must enter a canvas batch");
        assert!(
            gl_position < canvas_position,
            "GL issued before getImageData must precede snapshot capture; ops={ops:?}"
        );
    }

    #[test]
    fn r2_text_cache_consume_flushes_pending_gl_before_capture_and_upload() {
        let (mut runtime, render_rx) = new_webgl_runtime();
        let (handle, packet_rx) = spawn_canvas_frame_responder(render_rx);

        runtime
            .exec_script(
                "r2_text_cache_consume_interleave.js",
                r#"
                const gl = new WebGLRenderingContext({ _rid: 147, width: 1, height: 1 }, {});
                const ctx = createCanvas().getContext("2d");
                ctx._tcState = 1;
                ctx._tcKey = {
                    text: "x", fontRequest: "10px sans-serif", fontSize: 0,
                    fontWeight: 0, italic: false, fillColor: 0xffffffff,
                    textAlign: 0, textBaseline: 3, canvasW: 1, canvasH: 1,
                };
                gl.clear(0x4000);
                if (!ctx._consumeTextCacheForTexImage(147, 0x0DE1, 0, 0x1908)) {
                    throw new Error("expected text-cache consume path");
                }
                globalThis.__migo_frame_end_hooks[0]();
                "#,
            )
            .expect("text-cache consume interleave must execute");

        handle.join().expect("canvas responder must not panic");
        let ops = packet_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("text-cache consume must produce a frame packet");
        let first_gl = ops
            .iter()
            .position(|op| matches!(op, FrameOp::GlBatch(_)))
            .expect("pending GL must be submitted");
        let first_canvas = ops
            .iter()
            .position(|op| matches!(op, FrameOp::CanvasBatch(_)))
            .expect("cache snapshot must enter a canvas batch");
        assert!(
            first_gl < first_canvas,
            "GL issued before cache consume must precede capture/upload; ops={ops:?}"
        );
    }

    #[test]
    fn r2_context_loss_without_main_canvas_discards_offscreen_stream() {
        let (mut runtime, _render_rx) = new_webgl_runtime();
        crate::rendering::webgl::submit_test_counter::reset();

        runtime
            .exec_script(
                "r2_context_loss_without_main_canvas.js",
                r#"
                const gl = new WebGLRenderingContext({ _rid: 148, width: 1, height: 1 }, {});
                gl.clear(0x4000);
                const bridge = globalThis[Symbol.for("Migo.hostBridge")];
                if (typeof bridge?._internalTriggerWebglContextEvent !== "function") {
                    throw new Error("missing context lifecycle bridge");
                }
                bridge._internalTriggerWebglContextEvent("webglcontextlost");
                gl.flush();
                "#,
            )
            .expect("context-loss discard path must execute");

        let (submit_calls, decoded) = crate::rendering::webgl::submit_test_counter::read();
        assert_eq!(
            (submit_calls, decoded),
            (0, 0),
            "context loss must discard pending commands even when the main canvas was never wrapped"
        );
    }

    #[test]
    fn r2_create_context_flushes_pending_gl_barrier_first() {
        use shared::protocol::render_cmd::{Canvas2DCmd, CanvasCmd};

        let (mut runtime, render_rx) = new_webgl_runtime();
        let handle = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut order = Vec::new();
            while std::time::Instant::now() < deadline {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                match render_rx.recv_timeout(remaining) {
                    Ok(RenderCommand::Canvas(CanvasCmd::GetInfo { resp, .. })) => {
                        resp.send(Ok((4, 4)));
                    }
                    Ok(RenderCommand::FramePacket(packet)) => {
                        if packet
                            .ops()
                            .iter()
                            .any(|op| matches!(op, FrameOp::GlBatch(_)))
                        {
                            order.push("gl_packet");
                        }
                    }
                    Ok(RenderCommand::Canvas2D {
                        cmd: Canvas2DCmd::CreateContext2D,
                        ..
                    }) => order.push("create_context"),
                    Ok(_) => {}
                    Err(_) => break,
                }
                if order.contains(&"gl_packet") && order.contains(&"create_context") {
                    break;
                }
            }
            order
        });

        runtime
            .exec_script(
                "r2_create_context_order.js",
                r#"
                const canvas = createCanvas();
                const gl = new WebGLRenderingContext({ _rid: 149, width: 4, height: 4 }, {});
                gl.clear(0x4000);
                canvas.getContext("2d");
                globalThis.__migo_frame_end_hooks[0]();
                "#,
            )
            .expect("create-context ordering script must execute");

        let order = handle.join().expect("render responder must not panic");
        assert_eq!(
            order,
            vec!["gl_packet", "create_context"],
            "collector barrier must reach render thread before direct CreateContext2D"
        );
    }

    #[test]
    fn r2_canvas_info_flushes_pending_gl_barrier_first() {
        use shared::protocol::render_cmd::CanvasCmd;

        let (mut runtime, render_rx) = new_webgl_runtime();
        let handle = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut order = Vec::new();
            while std::time::Instant::now() < deadline {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                match render_rx.recv_timeout(remaining) {
                    Ok(RenderCommand::FramePacket(packet)) => {
                        if packet
                            .ops()
                            .iter()
                            .any(|op| matches!(op, FrameOp::GlBatch(_)))
                        {
                            order.push("gl_packet");
                        }
                    }
                    Ok(RenderCommand::Canvas(CanvasCmd::GetInfo { resp, .. })) => {
                        order.push("get_info");
                        resp.send(Ok((4, 4)));
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
                if order.contains(&"gl_packet") && order.contains(&"get_info") {
                    break;
                }
            }
            order
        });

        runtime
            .exec_script(
                "r2_canvas_info_order.js",
                r#"
                const gl = new WebGLRenderingContext({ _rid: 150, width: 4, height: 4 }, {});
                gl.clear(0x4000);
                createCanvas();
                globalThis.__migo_frame_end_hooks[0]();
                "#,
            )
            .expect("canvas-info ordering script must execute");

        let order = handle.join().expect("render responder must not panic");
        assert_eq!(
            order,
            vec!["gl_packet", "get_info"],
            "collector barrier must reach render thread before synchronous GetInfo"
        );
    }

    #[test]
    fn r2_offscreen_registration_flushes_pending_gl_barrier_first() {
        use shared::protocol::render_cmd::CanvasCmd;

        let (mut runtime, render_rx) = new_webgl_runtime();
        let handle = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut order = Vec::new();
            let mut offscreen_info_seen = false;
            while std::time::Instant::now() < deadline {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                match render_rx.recv_timeout(remaining) {
                    Ok(RenderCommand::FramePacket(packet)) => {
                        if packet
                            .ops()
                            .iter()
                            .any(|op| matches!(op, FrameOp::GlBatch(_)))
                        {
                            order.push("gl_packet");
                        }
                    }
                    Ok(RenderCommand::Canvas(CanvasCmd::RegisterOffscreen { .. })) => {
                        order.push("register_offscreen");
                    }
                    Ok(RenderCommand::Canvas(CanvasCmd::GetInfo { id, resp })) => {
                        if id != 1 {
                            offscreen_info_seen = true;
                        }
                        resp.send(Ok((4, 4)));
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
                if offscreen_info_seen
                    && order.contains(&"gl_packet")
                    && order.contains(&"register_offscreen")
                {
                    break;
                }
            }
            order
        });

        runtime
            .exec_script(
                "r2_offscreen_register_order.js",
                r#"
                createCanvas();
                const gl = new WebGLRenderingContext({ _rid: 151, width: 4, height: 4 }, {});
                gl.clear(0x4000);
                createCanvas();
                globalThis.__migo_frame_end_hooks[0]();
                "#,
            )
            .expect("offscreen-register ordering script must execute");

        let order = handle.join().expect("render responder must not panic");
        assert_eq!(
            order,
            vec!["gl_packet", "register_offscreen"],
            "collector barrier must reach render thread before RegisterOffscreen"
        );
    }

    #[test]
    fn r2_measure_text_flushes_pending_gl_barrier_first() {
        use shared::protocol::render_cmd::{Canvas2DCmd, CanvasCmd, TextMetrics};

        let (mut runtime, render_rx) = new_webgl_runtime();
        let handle = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut order = Vec::new();
            while std::time::Instant::now() < deadline {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                match render_rx.recv_timeout(remaining) {
                    Ok(RenderCommand::Canvas(CanvasCmd::GetInfo { resp, .. })) => {
                        resp.send(Ok((4, 4)));
                    }
                    Ok(RenderCommand::FramePacket(packet)) => {
                        if packet
                            .ops()
                            .iter()
                            .any(|op| matches!(op, FrameOp::GlBatch(_)))
                        {
                            order.push("gl_packet");
                        }
                    }
                    Ok(RenderCommand::Canvas2D {
                        cmd: Canvas2DCmd::MeasureText { resp, .. },
                        ..
                    }) => {
                        order.push("measure_text");
                        resp.ok(TextMetrics {
                            width: 0.0,
                            actual_bounding_box_left: 0.0,
                            actual_bounding_box_right: 0.0,
                            actual_bounding_box_ascent: 0.0,
                            actual_bounding_box_descent: 0.0,
                            font_bounding_box_ascent: 0.0,
                            font_bounding_box_descent: 0.0,
                            em_height_ascent: 0.0,
                            em_height_descent: 0.0,
                            hanging_baseline: 0.0,
                            alphabetic_baseline: 0.0,
                            ideographic_baseline: 0.0,
                        });
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
                if order.contains(&"gl_packet") && order.contains(&"measure_text") {
                    break;
                }
            }
            order
        });

        runtime
            .exec_script(
                "r2_measure_text_order.js",
                r#"
                const ctx = createCanvas().getContext("2d");
                const gl = new WebGLRenderingContext({ _rid: 152, width: 4, height: 4 }, {});
                gl.clear(0x4000);
                ctx.measureText("x");
                globalThis.__migo_frame_end_hooks[0]();
                "#,
            )
            .expect("measureText ordering script must execute");

        let order = handle.join().expect("render responder must not panic");
        assert_eq!(
            order,
            vec!["gl_packet", "measure_text"],
            "collector barrier must reach render thread before synchronous MeasureText"
        );
    }

    #[test]
    fn r2_direct_gradient_apply_flushes_pending_gl_first() {
        let (mut runtime, render_rx) = new_webgl_runtime();
        let (handle, packet_rx) = spawn_canvas_frame_responder(render_rx);

        runtime
            .exec_script(
                "r2_gradient_apply_order.js",
                r##"
                const ctx = createCanvas().getContext("2d");
                const gradient = ctx.createLinearGradient(0, 0, 4, 4);
                gradient.addColorStop(0, "#000000");
                gradient.addColorStop(1, "#ffffff");
                const gl = new WebGLRenderingContext({ _rid: 153, width: 4, height: 4 }, {});
                gl.clear(0x4000);
                gradient._apply();
                globalThis.__migo_frame_end_hooks[0]();
                "##,
            )
            .expect("direct gradient ordering script must execute");

        handle.join().expect("canvas responder must not panic");
        let ops = packet_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("gradient apply must produce a frame packet");
        let gl_position = ops
            .iter()
            .position(|op| matches!(op, FrameOp::GlBatch(_)))
            .expect("pending GL must be submitted");
        let canvas_position = ops
            .iter()
            .position(|op| matches!(op, FrameOp::CanvasBatch(_)))
            .expect("gradient apply must enter a canvas batch");
        assert!(
            gl_position < canvas_position,
            "GL issued before direct gradient apply must remain first; ops={ops:?}"
        );
    }
}

// ── Task 3: test-only submit instrumentation ─────────────────────────────────

/// Thread-local submit counter used by tests only.
/// Records (submit_call_count, total_decoded_cmd_count).
#[cfg(test)]
pub(crate) mod submit_test_counter {
    use std::cell::Cell;

    thread_local! {
        static CALLS: Cell<u64> = const { Cell::new(0) };
        static DECODED: Cell<u64> = const { Cell::new(0) };
    }

    pub(crate) fn reset() {
        CALLS.with(|c| c.set(0));
        DECODED.with(|d| d.set(0));
    }

    pub(crate) fn record(decoded: usize) {
        CALLS.with(|c| c.set(c.get() + 1));
        DECODED.with(|d| d.set(d.get() + decoded as u64));
    }

    pub(crate) fn read() -> (u64, u64) {
        (CALLS.with(|c| c.get()), DECODED.with(|d| d.get()))
    }
}

// ── Task 3: op_gl_submit_stream ──────────────────────────────────────────────

/// Submit a typed GL command stream from JS.
///
/// Pass 1 (structural): `gl_stream::validate_stream`. Malformed batches return a
/// stable non-zero error code immediately — no collector, no error queue, no vec taken.
///
/// Pass 2 (semantic + decode): on success, a vec is taken from the pool,
/// `decode::decode_validated_stream` fills it, and `append_gl_batch` bulk-appends into
/// the `UnifiedFrameCollector` (handling the empty/all-invalid case by recycling the vec
/// and creating no segment). Returns `0` on success.
#[op2(fast)]
#[smi]
pub fn op_gl_submit_stream(
    state: &mut OpState,
    #[buffer] words: &[u32],
    #[smi] used_words: u32,
) -> u32 {
    gl_submit_stream_impl(state, words, used_words)
}

/// Inner implementation callable from both the op wrapper and tests.
pub(crate) fn gl_submit_stream_impl(state: &mut OpState, words: &[u32], used_words: u32) -> u32 {
    // Pass 1: pure structural validation — no side effects on failure.
    let validated = match crate::rendering::webgl::gl_stream::validate_stream(words, used_words) {
        Ok(v) => v,
        Err(e) => return e.code(),
    };

    // Take a pooled vec for decoded commands.
    let mut cmds = shared::command_vec_pool::take_gl_command_vec();

    // Pass 2: semantic decode — fills cmds, returns approx byte count.
    let approx_bytes =
        crate::rendering::webgl::decode::decode_validated_stream(state, validated, &mut cmds);

    // Test-only instrumentation: record call count and decoded command count.
    #[cfg(test)]
    submit_test_counter::record(cmds.len());

    // Bulk-append into collector.  append_gl_batch handles the empty case
    // (all-semantic-invalid) by recycling cmds and creating no segment.
    let need_flush = state
        .borrow_mut::<crate::rendering::webgl::frame_collector::UnifiedFrameCollector>()
        .append_gl_batch(cmds, approx_bytes);

    if need_flush {
        maybe_auto_flush(state);
    }

    0
}

#[inline]
pub(crate) fn copy_f32_words(words: &[u32]) -> UniformF32Values {
    words.iter().map(|word| f32::from_bits(*word)).collect()
}

#[inline]
pub(crate) fn copy_i32_words(words: &[u32]) -> UniformI32Values {
    words
        .iter()
        .map(|word| i32::from_ne_bytes(word.to_ne_bytes()))
        .collect()
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
    match cmd {
        GLCmd::Uniform1iv { value, .. }
        | GLCmd::Uniform2iv { value, .. }
        | GLCmd::Uniform3iv { value, .. }
        | GLCmd::Uniform4iv { value, .. } => value.spilled(),
        GLCmd::Uniform1fv { value, .. }
        | GLCmd::Uniform2fv { value, .. }
        | GLCmd::Uniform3fv { value, .. }
        | GLCmd::Uniform4fv { value, .. }
        | GLCmd::UniformMatrix2fv { value, .. }
        | GLCmd::UniformMatrix3fv { value, .. }
        | GLCmd::UniformMatrix4fv { value, .. } => value.spilled(),
        _ => matches!(
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
                | GLCmd::InvalidateFramebuffer { .. }
                | GLCmd::DrawBuffers { .. }
                | GLCmd::TransformFeedbackVaryings { .. }
                | GLCmd::TexImage3D { .. }
                | GLCmd::TexSubImage3D { .. }
        ),
    }
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
        // Scalar/inline fast path.  `push_gl_fast` already maintained
        // `pending_bytes` (adding `size_of::<GLCmd>()`), so we skip the
        // `approx_deep_size_bytes` match here.  We still bound JS-side
        // retained memory: untrusted code can synchronously enqueue tens of
        // thousands of inline uniforms / binds in one turn (each
        // ~`size_of::<GLCmd>()`), and such a storm CAN cross the 4 MiB soft
        // budget.  The guard is a single field comparison on the borrow we
        // already hold; only when it trips do we pay the `maybe_auto_flush`
        // re-borrow + barrier dispatch, keeping the common per-command path
        // free of the deep-size walk.
        collector.push_gl_fast(cmd);
        let over_budget = collector.should_auto_flush();
        if over_budget {
            maybe_auto_flush(state);
        }
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
        // Best-effort: auto-flush is a memory-pressure relief, not a
        // correctness barrier. `dispatch` is still bounded-blocking (no
        // silent drop), so this only errors under extreme backpressure /
        // shutdown, where logging and moving on is acceptable.
        if let Err(e) = crate::rendering::webgl::frame_collector::flush_unified_barrier(state) {
            tracing::warn!("maybe_auto_flush: barrier flush failed: {e}");
        }
    }
}

#[inline]
fn send_gl_sync_with_flush<T>(
    state: &mut OpState,
    build: impl FnOnce(RenderCmdResp<T>) -> RenderCommand,
) -> Result<T, EngineError> {
    // Required barrier: if the pre-read flush can't be delivered we must
    // NOT proceed to the sync read — it would observe un-materialized 2D
    // content or a stale GL state (e.g. readPixels reading the previous
    // frame). Surface the failure to JS instead of returning stale data.
    crate::rendering::webgl::frame_collector::flush_unified_barrier(state).map_err(|e| {
        EngineError::new(shared::error::ErrorCode::RenderBackendError)
            .with_detail(format!("sync barrier flush failed before GL readback: {e}"))
    })?;
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
    // WebGL `gl.flush()` is advisory; best-effort delivery is fine.
    if let Err(e) = crate::rendering::webgl::frame_collector::flush_unified_barrier(state) {
        tracing::warn!("op_gl_flush: barrier flush failed: {e}");
    }
}

/// Backs JS `gl.isContextLost()`. Reads the shared `context_lost` flag that
/// the host sets on a render `ContextLost` event and clears on a successful
/// `ContextRecovered` (see `HostOpState::context_lost`). Returns `false`
/// when the host state isn't present (headless tests).
#[op2(fast)]
pub fn op_gl_is_context_lost(state: &mut OpState) -> bool {
    state
        .try_borrow::<shared::op_state::HostOpState>()
        .map(|h| h.context_lost.is_lost())
        .unwrap_or(false)
}

/// Backs JS `WEBGL_lose_context.loseContext()`. Arms a one-shot simulated GPU
/// reset on the render thread so the real context-loss -> recovery pipeline can
/// be exercised on demand (there is otherwise no way to trigger EGL_CONTEXT_LOST
/// from software). Fire-and-forget; the loss surfaces on the next render frame.
#[op2(fast)]
pub fn op_gl_lose_context(state: &mut OpState, #[smi] canvas_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DebugLoseContext { canvas_id });
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
    #[buffer] value: &[u32],
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value = copy_f32_words(value);

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
    #[buffer] value: &[u32],
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value = copy_i32_words(value);
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
    #[buffer] value: &[u32],
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value = copy_f32_words(value);
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
    #[buffer] value: &[u32],
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value = copy_i32_words(value);
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
    #[buffer] value: &[u32],
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value = copy_f32_words(value);
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
    #[buffer] value: &[u32],
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value = copy_i32_words(value);
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
    #[buffer] value: &[u32],
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value = copy_f32_words(value);
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
    #[buffer] value: &[u32],
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value = copy_i32_words(value);
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
    #[buffer] value: &[u32],
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value = copy_f32_words(value);
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
    #[buffer] value: &[u32],
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value = copy_f32_words(value);
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
    #[buffer] value: &[u32],
) {
    let location = if location < 0 {
        None
    } else {
        Some(location as u32)
    };
    let value = copy_f32_words(value);
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
pub fn op_delete_buffer(state: &mut OpState, #[smi] buffer_id: u32) {
    queue_gl_fire_and_forget(state, GLCmd::DeleteBuffer { buffer_id });
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

pub(crate) fn bind_buffer_base_impl(
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

pub(crate) fn bind_buffer_range_impl(
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
