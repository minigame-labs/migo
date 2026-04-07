use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use crate::{
    damage_effect::DamageEffect,
    dirty_region,
    frame_scheduler::FrameScheduler,
    global_fonts_mut, onscreen_window_from_surface,
    render_server::RenderServer,
    surface_system::SurfaceSystem,
    CanvasHandler, CanvasManager, FontData, Renderer2d, RendererGL,
};
use crossbeam_channel::{select, tick, Receiver};
use glow::HasContext;
use shared::error::{EngineError, EngineResult, ErrorCode};
use shared::protocol::render_cmd::{CanvasBatchPayload, GlBatchPayload, RenderCommand};
use shared::render_command_sender::CommandSender;
use shared::surface::SurfaceRef;
use shared::{FrameOp, FramePacket};
use tracing::{error, info, warn};

use shared::raf_signal::RafSender;

pub struct RenderThread {
    cmd_tx: CommandSender,
    handle: Option<thread::JoinHandle<()>>,
}

struct VsyncFrameDecision {
    should_signal_raf: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    should_present: bool,
    raf_time_ms: f64,
}

fn canvas2d_batch_should_mark_present_dirty(
    canvas_id: u32,
    batch_dirty: bool,
    present: bool,
    has_dirty_rect: bool,
) -> bool {
    present && canvas_id == 1 && (batch_dirty || has_dirty_rect)
}

/// Intermediate result from evaluating whether scissor optimization applies.
struct BatchScissorSetup {
    region: dirty_region::DirtyRegion,
    canvas_w: i32,
    canvas_h: i32,
}

/// Evaluate whether a canvas batch warrants scissor optimization.
///
/// Returns `Some` with the computed dirty region (in GL bottom-left-origin
/// coordinates) and canvas dimensions when all preconditions are met:
/// `present` is true, a dirty rect is provided, and canvas size is known.
///
/// The caller is still responsible for ensuring the target canvas is the
/// current GL context before issuing any GL scissor calls.
fn prepare_batch_scissor(
    present: bool,
    dirty_rect: &Option<shared::protocol::render_cmd::DirtyRect>,
    canvas_size: Option<(u32, u32)>,
) -> Option<BatchScissorSetup> {
    if !present {
        return None;
    }
    let rect = dirty_rect.as_ref()?;
    let (cw, ch) = canvas_size?;
    let cw = cw as i32;
    let ch = ch as i32;
    Some(BatchScissorSetup {
        region: dirty_region::DirtyRegion {
            x: rect.x.floor() as i32,
            y: (ch - (rect.y + rect.height).ceil() as i32).max(0),
            width: rect.width.ceil() as i32,
            height: rect.height.ceil() as i32,
        },
        canvas_w: cw,
        canvas_h: ch,
    })
}

fn mark_surface_destroyed(surface: &mut SurfaceSystem) {
    surface.on_surface_destroyed();
}

fn execute_canvas_batch(
    cm: &mut CanvasManager,
    gl: &glow::Context,
    renderer_2d: &mut Renderer2d,
    payload: CanvasBatchPayload,
) -> bool {
    use crate::damage_effect::DamageEffect;
    use crate::renderer2d::handler::classify_draw_damage;

    let canvas_id = payload.canvas_id;
    let commands = payload.commands;
    let present = payload.present;
    let mut batch_dirty = false;
    let is_onscreen = canvas_id == shared::protocol::render_cmd::CanvasId::from(1u32);

    // Layer 2 scissor trust gate: if Canvas2D state has non-identity
    // transform or active shadow, the JS-side dirty hint may underestimate
    // the actual draw area. Discard it to prevent scissor from clipping.
    let dirty_rect = {
        use crate::renderer2d::handler::state_allows_partial;
        let state_safe = cm
            .get_2d_context_mut(canvas_id)
            .map(|ctx| state_allows_partial(&ctx.state))
            .unwrap_or(true); // no context yet → no draws → hint is fine
        if state_safe {
            payload.dirty_rect
        } else {
            None
        }
    };

    let scissor_applied = if let Some(setup) =
        prepare_batch_scissor(present, &dirty_rect, cm.get_canvas_size(canvas_id).ok())
    {
        // Bind the target canvas before touching GL state so scissor
        // hits the correct EGL context / FBO.
        //
        // NOTE: invalidate_outside_dirty() was previously called here to
        // hint the tiled GPU that clean strips need not be loaded from
        // memory.  Removed because the DrawingBuffer path later does a
        // full-surface blit (blit_to_surface), which would read those
        // invalidated (now-undefined) regions and copy garbage to the
        // window surface on drivers that aggressively discard tiles
        // (Mali, PowerVR).
        let applied = if cm.make_current_needed(canvas_id).is_ok() {
            dirty_region::apply_scissor(gl, &setup.region, setup.canvas_w, setup.canvas_h);
            true
        } else {
            false
        };
        applied
    } else {
        false
    };

    for cmd in commands {
        // Classify damage BEFORE handle_command moves the cmd.
        // Reads Canvas2D state (transform, shadow) from the render thread —
        // this is the authoritative damage source, not the JS-side hint.
        let damage = if is_onscreen {
            cm.get_2d_context_mut(canvas_id)
                .map(|ctx| classify_draw_damage(&cmd, &ctx.state))
                .unwrap_or(DamageEffect::NoDamage)
        } else {
            DamageEffect::NoDamage
        };

        match renderer_2d.handle_command(cm, canvas_id, cmd) {
            Ok(was_render) => {
                if was_render {
                    batch_dirty = true;
                    cm.add_damage(damage);
                }
            }
            Err(e) => error!("Canvas2DBatch cmd failed: {}", e),
        }
    }

    if scissor_applied {
        debug_assert!(
            cm.current_canvas_id() == Some(canvas_id),
            "clear_scissor target mismatch: expected canvas {canvas_id:?} to be current"
        );
        dirty_region::clear_scissor(gl);
    }
    if batch_dirty {
        cm.mark_2d_dirty(canvas_id);
    }

    canvas2d_batch_should_mark_present_dirty(canvas_id, batch_dirty, present, dirty_rect.is_some())
}

fn execute_gl_batch(
    cm: &mut CanvasManager,
    gl: &glow::Context,
    renderer_gl: &mut RendererGL,
    payload: GlBatchPayload,
) -> bool {
    let commands = payload.commands;
    let cmd_count = commands.len();
    let mut batch_hit_onscreen = false;
    let mut error_count: u32 = 0;

    for gl_cmd in commands {
        match renderer_gl.handle_command(cm, gl, gl_cmd) {
            Ok(effect) => {
                if !matches!(effect, DamageEffect::NoDamage) {
                    batch_hit_onscreen = true;
                }
                cm.add_damage(effect);
            }
            Err(e) => {
                if error_count == 0 {
                    error!("GLBatch cmd failed: {}", e);
                }
                error_count += 1;
            }
        }
    }

    if error_count > 1 {
        warn!("GLBatch: {error_count}/{cmd_count} commands failed");
    }

    batch_hit_onscreen
}

/// Execute a FramePacket using caller-provided callbacks for each batch type.
/// Used by tests to verify packet structure and ordering without a real GL context.
/// Production code uses `execute_frame_packet` which handles `Materialize` directly.
#[cfg(test)]
pub(crate) fn execute_frame_packet_with_present_tracking<S, FC, FG>(
    packet: FramePacket,
    state: &mut S,
    mut on_canvas: FC,
    mut on_gl: FG,
) -> bool
where
    FC: FnMut(&mut S, CanvasBatchPayload) -> bool,
    FG: FnMut(&mut S, GlBatchPayload) -> bool,
{
    let mut should_present = false;

    for op in packet.into_ops() {
        match op {
            FrameOp::BeginFrame | FrameOp::Present | FrameOp::Materialize { .. } => {}
            FrameOp::CanvasBatch(payload) => {
                should_present |= on_canvas(state, payload);
            }
            FrameOp::GlBatch(payload) => {
                should_present |= on_gl(state, payload);
            }
        }
    }

    should_present
}

#[cfg(test)]
fn execute_frame_packet_with_present_tracking_for_test<S, FC, FG>(
    packet: FramePacket,
    state: &mut S,
    on_canvas: FC,
    on_gl: FG,
) -> bool
where
    FC: FnMut(&mut S, CanvasBatchPayload) -> bool,
    FG: FnMut(&mut S, GlBatchPayload) -> bool,
{
    execute_frame_packet_with_present_tracking(packet, state, on_canvas, on_gl)
}

fn execute_frame_packet(
    cm: &mut CanvasManager,
    gl: &glow::Context,
    renderer_2d: &mut Renderer2d,
    renderer_gl: &mut RendererGL,
    packet: FramePacket,
) -> bool {
    let mut should_present = false;

    for op in packet.into_ops() {
        match op {
            FrameOp::BeginFrame | FrameOp::Present => {}
            FrameOp::Materialize { canvas_id } => {
                // Flush femtovg so subsequent GL ops see Canvas2D results.
                // Also clear dirty_2d to prevent double-flush at present time.
                if cm.make_current_needed(canvas_id).is_ok() {
                    if let Ok(ctx) = cm.get_2d_context_mut(canvas_id) {
                        ctx.flush();
                    }
                    renderer_2d.clear_dirty_layer(canvas_id);
                    cm.clear_2d_dirty(canvas_id);
                }
            }
            FrameOp::CanvasBatch(payload) => {
                should_present |= execute_canvas_batch(cm, gl, renderer_2d, payload);
            }
            FrameOp::GlBatch(payload) => {
                should_present |= execute_gl_batch(cm, gl, renderer_gl, payload);
            }
        }
    }

    should_present
}

fn next_vsync_frame_decision(
    scheduler: &mut FrameScheduler,
    surface: &SurfaceSystem,
    frame_time_ms: f64,
) -> VsyncFrameDecision {
    let decision = scheduler.on_vsync(frame_time_ms);
    VsyncFrameDecision {
        should_signal_raf: decision.should_render,
        should_present: decision.should_render && surface.can_present(),
        raf_time_ms: decision.raf_time_ms,
    }
}

#[cfg(test)]
fn finalize_vsync_frame_decision<F>(
    scheduler: &mut FrameScheduler,
    surface: &mut SurfaceSystem,
    frame_time_ms: f64,
    apply_pending: F,
) -> VsyncFrameDecision
where
    F: FnOnce(&mut SurfaceSystem),
{
    let decision = scheduler.on_vsync(frame_time_ms);
    apply_pending(surface);
    VsyncFrameDecision {
        should_signal_raf: decision.should_render,
        should_present: decision.should_render && surface.can_present(),
        raf_time_ms: decision.raf_time_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        canvas2d_batch_should_mark_present_dirty,
        execute_frame_packet_with_present_tracking_for_test, finalize_vsync_frame_decision,
        mark_surface_destroyed, next_vsync_frame_decision,
    };
    use crate::{frame_scheduler::FrameScheduler, SurfaceSystem};
    use shared::protocol::render_cmd::{Canvas2DCmd, CanvasBatchPayload, DirtyRect};
    use shared::{FrameOp, FramePacketBuilder};

    #[test]
    fn vsync_path_uses_scheduler_and_surface_state_for_presentation() {
        let mut scheduler = FrameScheduler::new(60);
        let mut surface = SurfaceSystem::new();

        let first = next_vsync_frame_decision(&mut scheduler, &surface, 0.0);
        assert!(first.should_signal_raf);
        assert!(!first.should_present);
        assert_eq!(first.raf_time_ms, 0.0);

        surface.on_surface_available((1080, 1920));

        let second = next_vsync_frame_decision(&mut scheduler, &surface, 16.667);
        assert!(second.should_signal_raf);
        assert!(second.should_present);
        assert!(second.raf_time_ms > 0.0);
    }

    #[test]
    fn surface_destroyed_blocks_presentation_until_surface_recreated() {
        let mut scheduler = FrameScheduler::new(60);
        let mut surface = SurfaceSystem::new();
        surface.on_surface_available((1080, 1920));

        let first = next_vsync_frame_decision(&mut scheduler, &surface, 0.0);
        assert!(first.should_present);

        mark_surface_destroyed(&mut surface);

        let second = next_vsync_frame_decision(&mut scheduler, &surface, 16.667);
        assert!(second.should_signal_raf);
        assert!(!second.should_present);
    }

    #[test]
    fn pending_surface_destroyed_prevents_presentation_in_same_vsync_iteration() {
        let mut scheduler = FrameScheduler::new(60);
        let mut surface = SurfaceSystem::new();
        surface.on_surface_available((1080, 1920));

        let decision =
            finalize_vsync_frame_decision(&mut scheduler, &mut surface, 0.0, |surface| {
                mark_surface_destroyed(surface);
            });

        assert!(decision.should_signal_raf);
        assert!(!decision.should_present);
    }

    #[test]
    fn non_presenting_canvas2d_batch_does_not_mark_present_dirty() {
        assert!(!canvas2d_batch_should_mark_present_dirty(
            1, true, false, true
        ));
        assert!(canvas2d_batch_should_mark_present_dirty(
            1, true, true, true
        ));
        assert!(canvas2d_batch_should_mark_present_dirty(
            1, false, true, true
        ));
        assert!(!canvas2d_batch_should_mark_present_dirty(
            1, false, true, false
        ));
    }

    #[test]
    fn frame_packet_presenting_canvas_batch_without_present_op_still_requests_present() {
        let packet = FramePacketBuilder::new(1, 16.6)
            .push(FrameOp::BeginFrame)
            .push(FrameOp::CanvasBatch(CanvasBatchPayload {
                canvas_id: 1,
                commands: vec![Canvas2DCmd::Save],
                present: true,
                dirty_rect: None,
            }))
            .finish();

        let mut canvas_exec_count = 0;
        let mut gl_exec_count = 0;

        let should_present = execute_frame_packet_with_present_tracking_for_test(
            packet,
            &mut (),
            |(), payload| {
                canvas_exec_count += 1;
                assert!(payload.present);
                true
            },
            |(), _payload| {
                gl_exec_count += 1;
                false
            },
        );

        assert!(should_present);
        assert_eq!(canvas_exec_count, 1);
        assert_eq!(gl_exec_count, 0);
    }

    #[test]
    fn frame_packet_presenting_canvas_batch_executes_canvas_work_and_requests_present() {
        let packet = FramePacketBuilder::new(1, 16.6)
            .push(FrameOp::BeginFrame)
            .push(FrameOp::CanvasBatch(CanvasBatchPayload {
                canvas_id: 1,
                commands: vec![Canvas2DCmd::Save],
                present: true,
                dirty_rect: Some(DirtyRect {
                    x: 0.0,
                    y: 0.0,
                    width: 8.0,
                    height: 8.0,
                }),
            }))
            .push(FrameOp::Present)
            .finish();

        #[derive(Default)]
        struct PacketExecStats {
            canvas_exec_count: usize,
            gl_exec_count: usize,
        }

        let mut stats = PacketExecStats::default();

        let should_present = execute_frame_packet_with_present_tracking_for_test(
            packet,
            &mut stats,
            |stats, payload| {
                stats.canvas_exec_count += 1;
                assert_eq!(payload.canvas_id, 1);
                assert!(payload.present);
                assert_eq!(payload.commands.len(), 1);
                true
            },
            |stats, _payload| {
                stats.gl_exec_count += 1;
                false
            },
        );

        assert!(should_present);
        assert_eq!(stats.canvas_exec_count, 1);
        assert_eq!(stats.gl_exec_count, 0);
    }

    #[test]
    fn frame_packet_state_only_canvas_batch_does_not_request_present_from_explicit_boundary() {
        let packet = FramePacketBuilder::new(2, 16.6)
            .push(FrameOp::BeginFrame)
            .push(FrameOp::CanvasBatch(CanvasBatchPayload {
                canvas_id: 1,
                commands: vec![Canvas2DCmd::Save],
                present: true,
                dirty_rect: None,
            }))
            .push(FrameOp::Present)
            .finish();

        let should_present = execute_frame_packet_with_present_tracking_for_test(
            packet,
            &mut (),
            |(), payload| {
                assert!(payload.present);
                false
            },
            |(), _payload| {
                panic!("unexpected gl batch");
            },
        );

        assert!(!should_present);
    }

    #[test]
    fn frame_packet_gl_work_requests_present_without_explicit_present_op() {
        let packet = FramePacketBuilder::new(3, 16.6)
            .push(FrameOp::BeginFrame)
            .push(FrameOp::GlBatch(
                shared::protocol::render_cmd::GlBatchPayload {
                    commands: Vec::new(),
                },
            ))
            .finish();

        let should_present = execute_frame_packet_with_present_tracking_for_test(
            packet,
            &mut (),
            |(), _payload| {
                panic!("unexpected canvas batch");
            },
            |(), _payload| true,
        );

        assert!(should_present);
    }

    // ── prepare_batch_scissor tests ──

    #[test]
    fn prepare_batch_scissor_returns_none_when_not_presenting() {
        use super::prepare_batch_scissor;
        let rect = Some(DirtyRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        });
        assert!(prepare_batch_scissor(false, &rect, Some((800, 600))).is_none());
    }

    #[test]
    fn prepare_batch_scissor_returns_none_when_dirty_rect_is_none() {
        use super::prepare_batch_scissor;
        assert!(prepare_batch_scissor(true, &None, Some((800, 600))).is_none());
    }

    #[test]
    fn prepare_batch_scissor_returns_none_when_canvas_size_unavailable() {
        use super::prepare_batch_scissor;
        let rect = Some(DirtyRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        });
        assert!(prepare_batch_scissor(true, &rect, None).is_none());
    }

    #[test]
    fn prepare_batch_scissor_returns_region_when_all_conditions_met() {
        use super::prepare_batch_scissor;
        let rect = Some(DirtyRect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        });
        let setup = prepare_batch_scissor(true, &rect, Some((800, 600)));
        assert!(setup.is_some());
        let s = setup.unwrap();
        assert_eq!(s.canvas_w, 800);
        assert_eq!(s.canvas_h, 600);
        assert_eq!(s.region.x, 10);
        assert_eq!(s.region.width, 100);
        assert_eq!(s.region.height, 50);
    }

    #[test]
    fn prepare_batch_scissor_flips_y_from_top_left_to_gl_bottom_left_origin() {
        use super::prepare_batch_scissor;
        // DirtyRect: top-left origin, y=20, height=50 → bottom edge at y=70
        // GL scissor: bottom-left origin → y = canvas_h - bottom_edge = 600 - 70 = 530
        let rect = Some(DirtyRect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        });
        let s = prepare_batch_scissor(true, &rect, Some((800, 600))).unwrap();
        assert_eq!(s.region.y, 530);
    }

    #[test]
    fn prepare_batch_scissor_clamps_negative_y_to_zero() {
        use super::prepare_batch_scissor;
        // DirtyRect extends past canvas bottom: y=580, h=100 → bottom_edge=680
        // GL y = (600 - 680).max(0) = 0
        let rect = Some(DirtyRect {
            x: 0.0,
            y: 580.0,
            width: 100.0,
            height: 100.0,
        });
        let s = prepare_batch_scissor(true, &rect, Some((800, 600))).unwrap();
        assert_eq!(s.region.y, 0);
    }

    #[test]
    fn prepare_batch_scissor_ceils_fractional_dimensions() {
        use super::prepare_batch_scissor;
        let rect = Some(DirtyRect {
            x: 10.3,
            y: 20.7,
            width: 99.1,
            height: 49.9,
        });
        let s = prepare_batch_scissor(true, &rect, Some((800, 600))).unwrap();
        // x floors: 10.3 → 10
        assert_eq!(s.region.x, 10);
        // width ceils: 99.1 → 100
        assert_eq!(s.region.width, 100);
        // height ceils: 49.9 → 50
        assert_eq!(s.region.height, 50);
    }

    // ── Materialize / mixed-frame ordering tests ──

    #[test]
    fn frame_packet_materialize_op_does_not_affect_present_decision() {
        let packet = FramePacketBuilder::new(1, 16.6)
            .push(FrameOp::BeginFrame)
            .push(FrameOp::CanvasBatch(CanvasBatchPayload {
                canvas_id: 1,
                commands: vec![Canvas2DCmd::Save],
                present: true,
                dirty_rect: None,
            }))
            .push(FrameOp::Materialize { canvas_id: 1 })
            .push(FrameOp::GlBatch(
                shared::protocol::render_cmd::GlBatchPayload {
                    commands: Vec::new(),
                },
            ))
            .push(FrameOp::Present)
            .finish();

        let should_present = execute_frame_packet_with_present_tracking_for_test(
            packet,
            &mut (),
            |(), _payload| true,
            |(), _payload| false,
        );

        assert!(should_present);
    }

    #[test]
    fn frame_packet_executes_interleaved_ops_in_submission_order() {
        let packet = FramePacketBuilder::new(1, 16.6)
            .push(FrameOp::BeginFrame)
            .push(FrameOp::CanvasBatch(CanvasBatchPayload {
                canvas_id: 1,
                commands: vec![Canvas2DCmd::Save],
                present: false,
                dirty_rect: None,
            }))
            .push(FrameOp::Materialize { canvas_id: 1 })
            .push(FrameOp::GlBatch(
                shared::protocol::render_cmd::GlBatchPayload {
                    commands: Vec::new(),
                },
            ))
            .push(FrameOp::CanvasBatch(CanvasBatchPayload {
                canvas_id: 1,
                commands: vec![Canvas2DCmd::Restore],
                present: true,
                dirty_rect: None,
            }))
            .push(FrameOp::Present)
            .finish();

        let mut order: Vec<&'static str> = Vec::new();
        let should_present = execute_frame_packet_with_present_tracking_for_test(
            packet,
            &mut order,
            |order, _payload| {
                order.push("canvas");
                true
            },
            |order, _payload| {
                order.push("gl");
                false
            },
        );

        assert!(should_present);
        assert_eq!(order, vec!["canvas", "gl", "canvas"]);
    }
}

impl RenderThread {
    /// Spawn render thread.
    ///
    /// * `raf_tx` — frame timestamp sender (render → JS async op).
    ///   On Android this is eventfd-backed; on other platforms, tokio mpsc.
    /// * `vsync_rx` — optional crossbeam receiver for Choreographer VSync signals
    /// * `host_id` — host identifier for debug stats registry
    pub fn spawn(
        raf_tx: RafSender,
        vsync_rx: Option<Receiver<f64>>,
        host_id: i32,
        initial_surface: Option<SurfaceRef>,
        dpi: f32,
        app_cache_dir: Option<std::path::PathBuf>,
        gpu_caps: std::sync::Arc<shared::device::gpu_caps::GpuCaps>,
    ) -> EngineResult<Self> {
        let (cmd_tx, cmd_rx) = CommandSender::new();

        let handle = std::thread::Builder::new()
            .name("Migo-RenderThread".into())
            .spawn(move || {
                shared::thread_priority::set_current_thread_priority(
                    shared::thread_priority::Priority::Display,
                );
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // Both Android and non-Android use the same EGL library name.
                const EGL_LIB: &str = "libEGL.so";

                let mut cm = match CanvasManager::new_with_resource(EGL_LIB, dpi, app_cache_dir.as_deref(), &gpu_caps) {
                    Ok(c) => c,
                    Err(e) => {
                        gpu_caps.set_failed(format!("CanvasManager init failed: {}", e));
                        error!("CanvasManager init failed: {}", e);
                        return;
                    }
                };

                let has_initial_surface = initial_surface.is_some();
                let initial_surface_size = initial_surface.as_ref().map(|surface| surface.size());
                let mut initial_onscreen_ok = false;

                // If an initial surface is provided, create the onscreen context immediately.
                if let Some(surface) = initial_surface {
                    match onscreen_window_from_surface(surface.as_ref()) {
                        Ok(win) => {
                            if let Err(e) = cm.create_onscreen(win, None) {
                                error!("create_onscreen failed: {}", e);
                                gpu_caps.set_failed(format!("create_onscreen failed: {}", e));
                            } else {
                                initial_onscreen_ok = true;
                            }
                        }
                        Err(e) => {
                            gpu_caps.set_failed(format!("initial surface is not supported: {}", e));
                            error!("initial surface is not supported: {}", e)
                        }
                    }
                }

                // Create GL function loader.
                let gl = unsafe {
                    glow::Context::from_loader_function(|s| {
                        cm.egl
                            .get_proc_address(s)
                            .map(|f| f as *const std::ffi::c_void)
                            .unwrap_or(std::ptr::null())
                    })
                };

                // ---- Timing sources ----
                // When Choreographer VSync is available, use it as primary timing.
                // Otherwise fall back to a software ticker at the configured FPS.
                let has_vsync = vsync_rx.is_some();
                let vsync: Receiver<f64> = vsync_rx.unwrap_or_else(crossbeam_channel::never);

                let mut fps: u32 = 60;
                let mut ticker: Receiver<Instant> = if has_vsync {
                    crossbeam_channel::never()
                } else {
                    tick(Duration::from_secs_f32(1.0 / fps as f32))
                };
                let mut frame_scheduler = FrameScheduler::new(fps);

                let start_time = Instant::now();
                let mut dirty = true;
                let mut paused = false;
                let mut surface_system = SurfaceSystem::new();
                if initial_onscreen_ok {
                    if let Some(size) = initial_surface_size {
                        surface_system.on_surface_available(size);
                    } else if has_initial_surface {
                        surface_system.on_resume();
                    }
                }

                let mut canvas_handler = CanvasHandler::new();
                let mut renderer_2d = Renderer2d::new();
                let mut renderer_gl = RendererGL::new();
                let mut render_server = RenderServer::new();

                // ---- FPS stats ----
                let debug_stats = shared::stats::register_stats(host_id);
                let mut frame_count: u32 = 0;
                let mut fps_timer = Instant::now();
                let mut last_frame_time = Instant::now();
                let mut first_frame_recorded = false;

                // Deferred EGL context recovery flag. Set to true when
                // swap_buffers detects context loss (EGL_CONTEXT_LOST).
                // Recovery is deferred to the top of the next frame iteration
                // instead of running inline during present, because
                // try_recover_context() performs a full EGL surface+context
                // teardown and recreate, which is too expensive to run in the
                // time-critical swap path where it delays the RAF signal and
                // can cause cascading frame drops.
                let mut needs_context_recovery = false;

                enum LoopCtl {
                    Continue,
                    Shutdown,
                }

                let handle_one_cmd = |cmd: RenderCommand,
                                          cm: &mut CanvasManager,
                                          gl: &glow::Context,
                                          canvas_handler: &mut CanvasHandler,
                                          renderer_2d: &mut Renderer2d,
                                           renderer_gl: &mut RendererGL,
                                           fps: &mut u32,
                                           frame_scheduler: &mut FrameScheduler,
                                           ticker: &mut Receiver<Instant>,
                                           dirty: &mut bool,
                                           paused: &mut bool,
                                           has_vsync: bool,
                                           surface_system: &mut SurfaceSystem,
                                           render_server: &mut RenderServer|
                 -> LoopCtl {
                    match cmd {
                        RenderCommand::Shutdown => {
                            info!("RenderThread received Shutdown");
                            cm.destroy_all(&gl);
                            return LoopCtl::Shutdown;
                        }

                        RenderCommand::FrameRate(new_fps) => {
                            let new_fps = new_fps.clamp(1, 120);
                            if has_vsync {
                                frame_scheduler.set_preferred_fps(new_fps);
                                info!("RenderThread target fps changed to {} (scheduler)", new_fps);
                            } else if new_fps != *fps {
                                // Software ticker mode: recreate ticker at new interval.
                                *fps = new_fps;
                                if !*paused {
                                    *ticker = tick(Duration::from_secs_f32(1.0 / *fps as f32));
                                }
                                info!("RenderThread fps changed to {}", fps);
                            }
                        }

                        RenderCommand::Canvas(canvas_cmd) => {
                            // Only mark dirty for commands that affect onscreen visual output.
                            let is_recreate = matches!(&canvas_cmd, shared::protocol::render_cmd::CanvasCmd::RecreateOnscreen { .. });
                            let recreate_surface_size = match &canvas_cmd {
                                shared::protocol::render_cmd::CanvasCmd::RecreateOnscreen {
                                    surface,
                                    ..
                                } => Some(surface.size()),
                                _ => None,
                            };
                            let affects_onscreen = match &canvas_cmd {
                                shared::protocol::render_cmd::CanvasCmd::RecreateOnscreen { .. } => true,
                                shared::protocol::render_cmd::CanvasCmd::ResizeCanvas { id, .. } => *id == 1,
                                _ => false,
                            };
                            match canvas_handler.handle_command(cm, canvas_cmd) {
                                Ok(()) => {
                                    if is_recreate {
                                        if let Some(size) = recreate_surface_size {
                                            surface_system.on_surface_available(size);
                                        }
                                        info!("RenderThread surface recreated");
                                    }
                                    if affects_onscreen {
                                        *dirty = true;
                                    }
                                }
                                Err(e) => {
                                    error!("CanvasCmd failed: {}", e);
                                }
                            }
                        }

                        RenderCommand::GL(gl_cmd) => match renderer_gl.handle_command(cm, gl, gl_cmd) {
                            Ok(effect) => {
                                if !matches!(effect, DamageEffect::NoDamage) {
                                    *dirty = true;
                                }
                                cm.add_damage(effect);
                            }
                            Err(e) => error!("GLCmd failed: {}", e),
                        },
                        RenderCommand::GLBatch(payload) => {
                            if execute_gl_batch(cm, gl, renderer_gl, payload) {
                                *dirty = true;
                            }
                        }

                        RenderCommand::Canvas2D { canvas_id, cmd } => match renderer_2d.handle_command(cm, canvas_id, cmd) {
                            Ok(was_render) => {
                                if was_render {
                                    cm.mark_2d_dirty(canvas_id);
                                    // NOTE: dirty flag is NOT set here. Canvas2D commands
                                    // may arrive mid-frame (e.g. _frameEnd() before
                                    // measureText). Setting dirty here would cause the
                                    // render thread to present a partial frame on the next
                                    // VSync, producing visible flicker. Instead, the JS
                                    // frame-end (op_frame_end_all) sends an explicit
                                    // Invalidate to trigger the present.
                                }
                            }
                            Err(e) => error!("Canvas2D failed: {}", e),
                        },

                        // V2: Batched commands - process all commands in a single frame
                        RenderCommand::Canvas2DBatch(payload) => {
                            if execute_canvas_batch(cm, gl, renderer_2d, payload) {
                                *dirty = true;
                            }
                        }

                        RenderCommand::FramePacket(mut packet) => {
                            render_server.stamp_packet(&mut packet);
                            if execute_frame_packet(cm, gl, renderer_2d, renderer_gl, packet) {
                                *dirty = true;
                            }
                        }

                        // Invalidate signal for on-demand rendering
                        RenderCommand::Invalidate => {
                            *dirty = true;
                        }

                        RenderCommand::Pause => {
                            if !*paused {
                                *paused = true;
                                surface_system.on_pause();
                                if !has_vsync {
                                    *ticker = crossbeam_channel::never();
                                }
                                info!("RenderThread paused");
                            }
                        }

                        RenderCommand::Resume => {
                            if *paused {
                                *paused = false;
                                surface_system.on_resume();
                                if !has_vsync {
                                    *ticker = tick(Duration::from_secs_f32(1.0 / *fps as f32));
                                }
                                info!("RenderThread resumed");
                            }
                        }

                        RenderCommand::SurfaceDestroyed => {
                            mark_surface_destroyed(surface_system);
                        }

                        RenderCommand::LoadFont { key, bytes, resp } => {
                            // 1) Insert into global font store.
                            let data = FontData {
                                name: key.clone(),
                                bytes,
                            };
                            global_fonts_mut().insert(&key, data.clone());

                            // 2) Register in all existing canvas FontManagers.
                            for (_cid, ctx2d) in cm.contexts_2d_iter_mut() {
                                ctx2d.font_manager.register_font(&mut ctx2d.canvas, &key, &data);
                            }

                            info!("RenderThread: loaded font '{}'", key);
                            resp.ok(key);
                        }

                        RenderCommand::GetTextLineHeight { font_family, font_size, bold, italic, resp } => {
                            // Find any 2D context to measure with.
                            let result: f32 = if let Some((_cid, ctx2d)) = cm.contexts_2d_iter_mut().next() {
                                // Resolve font id for the requested family/style.
                                let font_id = ctx2d.font_manager.resolve_font_id(&font_family, bold, italic)
                                    .or_else(|| ctx2d.font_manager.default_font_id());

                                if let Some(fid) = font_id {
                                    let mut paint = femtovg::Paint::color(femtovg::Color::black());
                                    paint.set_font(&[fid]);
                                    paint.set_font_size(font_size);
                                    match ctx2d.canvas.measure_font(&paint) {
                                        Ok(fm) => fm.ascender() - fm.descender(),
                                        Err(_) => font_size * 1.2,
                                    }
                                } else {
                                    font_size * 1.2
                                }
                            } else {
                                // No canvas available, use common approximation.
                                font_size * 1.2
                            };
                            resp.ok(result);
                        }

                        _ => {}
                    }

                    LoopCtl::Continue
                };

                let drain_cmds = |cm: &mut CanvasManager,
                                      gl: &glow::Context,
                                      canvas_handler: &mut CanvasHandler,
                                      renderer_2d: &mut Renderer2d,
                                       renderer_gl: &mut RendererGL,
                                       fps: &mut u32,
                                       frame_scheduler: &mut FrameScheduler,
                                       ticker: &mut Receiver<Instant>,
                                       dirty: &mut bool,
                                       paused: &mut bool,
                                       surface_system: &mut SurfaceSystem,
                                       render_server: &mut RenderServer|
                 -> LoopCtl {
                    // Drain pending commands from the channel.
                    // Limit per drain to prevent frame-time spikes (~2-3 ms on ARM SoC).
                    const MAX_DRAIN: usize = 512;
                    for _ in 0..MAX_DRAIN {
                        match cmd_rx.try_recv() {
                            Ok(cmd) => {
                                match handle_one_cmd(cmd, cm, gl, canvas_handler, renderer_2d, renderer_gl, fps, frame_scheduler, ticker, dirty, paused, has_vsync, surface_system, render_server) {
                                    LoopCtl::Continue => {}
                                    LoopCtl::Shutdown => return LoopCtl::Shutdown,
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    LoopCtl::Continue
                };

                // Shared frame presentation + RAF signal logic.
                let present_frame_and_signal_raf = |cm: &mut CanvasManager,
                                                         renderer_2d: &mut Renderer2d,
                                                         dirty: &mut bool,
                                                         paused: bool,
                                                         should_present: bool,
                                                         ts: f64,
                                                         debug_stats: &shared::stats::DebugStats,
                                                         frame_count: &mut u32,
                                                        fps_timer: &mut Instant,
                                                        last_frame_time: &mut Instant,
                                                        first_frame_recorded: &mut bool,
                                                        needs_recovery: &mut bool| {
                    // Drain completed texture uploads from the upload thread
                    // and register them in the image registry for rendering.
                    let dropped_recoveries = cm.drain_upload_completed();
                    if dropped_recoveries > 0 {
                        debug_stats.dropped_upload_recoveries.fetch_add(dropped_recoveries, Ordering::Relaxed);
                    }
                    // Capture upload rejections before reset clears the counter.
                    let rejections = cm.take_upload_frame_rejections();
                    if rejections > 0 {
                        debug_stats.upload_frame_rejections.fetch_add(rejections, Ordering::Relaxed);
                    }
                    // Reset per-frame upload budget for the new frame.
                    cm.reset_frame_upload_budget();
                    // Signal RAF BEFORE swap so JS can prepare the next frame
                    // while the GPU waits for VSync.  Must be unconditional —
                    // JS may call requestAnimationFrame without drawing (dirty=false)
                    // and still needs the next timestamp to keep the loop alive.
                    if !paused && !raf_tx.signal(ts) {
                        debug_stats.dropped_frames.fetch_add(1, Ordering::Relaxed);
                    }

                    // Present the completed frame (only if we have a valid surface).
                    let did_swap = if *dirty && should_present {
                        let onscreen_id = shared::protocol::render_cmd::CanvasId::from(1u32);
                        let (canvas_w, canvas_h) = cm.get_canvas_size(onscreen_id).unwrap_or((0, 0));
                        let tracked_viewport = cm
                            .gl_state
                            .get(&onscreen_id)
                            .and_then(|s| s.viewport)
                            .unwrap_or((0, 0, canvas_w as i32, canvas_h as i32));
                        // Declare damage region BEFORE any onscreen rendering
                        // (femtovg flush, DrawingBuffer blit). Per EGL_KHR_partial_update
                        // spec, the declaration must precede GL draws to the main
                        // framebuffer so the driver can skip loading unchanged tiles.
                        cm.declare_frame_damage(onscreen_id);

                        match cm.flush_dirty_2d_contexts() {
                            Ok(flushed_ids) => {
                                for canvas_id in flushed_ids {
                                    renderer_2d.clear_dirty_layer(canvas_id);
                                }
                            }
                            Err(e) => {
                                warn!("flush_dirty_2d_contexts failed: {}", e);
                            }
                        }
                        unsafe {
                            gl.viewport(
                                tracked_viewport.0,
                                tracked_viewport.1,
                                tracked_viewport.2,
                                tracked_viewport.3,
                            )
                        };

                        let swap_ok = match cm.swap_buffers_no_restore(shared::protocol::render_cmd::CanvasId::from(1u32), true) {
                            Ok(resolved_damage) => {
                                use crate::dirty_region::damage_tracker::ResolvedDamage;
                                match resolved_damage {
                                    ResolvedDamage::Partial { width, height, .. } => {
                                        debug_stats.partial_damage_frames.fetch_add(1, Ordering::Relaxed);
                                        let kpx = ((width as u64) * (height as u64) / 1000) as u32;
                                        debug_stats.damage_area_k_pixels.fetch_add(kpx, Ordering::Relaxed);
                                    }
                                    ResolvedDamage::FullSurface => {
                                        debug_stats.full_surface_frames.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                true
                            }
                            Err(e) => {
                                warn!("swap_buffers_no_restore failed: {}", e);
                                if cm.is_context_lost() {
                                    *needs_recovery = true;
                                    warn!("EGL context lost, recovery deferred to next frame");
                                }
                                false
                            }
                        };
                        *dirty = false;

                        if swap_ok && !*first_frame_recorded {
                            *first_frame_recorded = true;
                            let first_ms = start_time.elapsed().as_millis() as u32;
                            debug_stats.first_frame_ms.store(first_ms, Ordering::Relaxed);
                        }
                        swap_ok
                    } else {
                        false
                    };

                    // FPS stats.
                    let now = Instant::now();
                    if did_swap {
                        let frame_dur = now.duration_since(*last_frame_time);
                        *last_frame_time = now;
                        debug_stats.frame_time_us.store(frame_dur.as_micros() as u32, Ordering::Relaxed);
                        *frame_count += 1;
                    }
                    let elapsed = fps_timer.elapsed();
                    if elapsed >= Duration::from_millis(500) {
                        let measured_fps = *frame_count as f32 / elapsed.as_secs_f32();
                        debug_stats.fps_x10.store((measured_fps * 10.0) as u32, Ordering::Relaxed);
                        *frame_count = 0;
                        *fps_timer = now;
                    }
                };

                loop {
                    // --- Deferred EGL context recovery ---
                    // Performed at the top of the frame loop where it is less
                    // timing-critical than inside the swap path. This avoids
                    // blocking the RAF signal with a full EGL teardown+recreate.
                    if needs_context_recovery {
                        needs_context_recovery = false;
                        match cm.try_recover_context() {
                            Ok(true) => info!("EGL context recovered at frame top, resuming rendering"),
                            Ok(false) => warn!("EGL context recovery deferred (no window handle)"),
                            Err(re) => warn!("EGL context recovery failed: {}", re),
                        }
                    }

                    select! {
                        recv(ticker) -> _ => {
                            // Software ticker path (non-Android fallback).
                            // Frame timing: drain → swap → RAF signal.
                            let ts = start_time.elapsed().as_secs_f64() * 1000.0;
                            render_server.set_raf_time_ms(ts);

                            // 1) Drain all pending commands from the previous frame.
                            match drain_cmds(&mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut frame_scheduler, &mut ticker, &mut dirty, &mut paused, &mut surface_system, &mut render_server) {
                                LoopCtl::Continue => {}
                                LoopCtl::Shutdown => {
                                    shared::stats::unregister_stats(host_id);
                                    return;
                                }
                            }

                            // 2) Present frame and signal RAF.
                            present_frame_and_signal_raf(&mut cm, &mut renderer_2d, &mut dirty, paused, surface_system.can_present(), ts, &debug_stats, &mut frame_count, &mut fps_timer, &mut last_frame_time, &mut first_frame_recorded, &mut needs_context_recovery);
                        }

                        recv(vsync) -> _msg => {
                            // Choreographer VSync path (Android).
                            let Some(frame_time_ms) = _msg.ok() else {
                                continue;
                            };

                            let decision = next_vsync_frame_decision(
                                &mut frame_scheduler,
                                &surface_system,
                                frame_time_ms,
                            );

                            if !decision.should_signal_raf {
                                continue;
                            }

                            render_server.set_raf_time_ms(decision.raf_time_ms);

                            // 1) Drain all pending commands.
                            match drain_cmds(&mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut frame_scheduler, &mut ticker, &mut dirty, &mut paused, &mut surface_system, &mut render_server) {
                                LoopCtl::Continue => {}
                                LoopCtl::Shutdown => {
                                    shared::stats::unregister_stats(host_id);
                                    return;
                                }
                            }

                            // 2) Present frame and signal RAF.
                            let should_present = decision.should_signal_raf && surface_system.can_present();
                            present_frame_and_signal_raf(&mut cm, &mut renderer_2d, &mut dirty, paused, should_present, decision.raf_time_ms, &debug_stats, &mut frame_count, &mut fps_timer, &mut last_frame_time, &mut first_frame_recorded, &mut needs_context_recovery);
                        }

                        recv(cmd_rx) -> msg => {
                            match msg {
                                Ok(cmd) => {
                                    match handle_one_cmd(cmd, &mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut frame_scheduler, &mut ticker, &mut dirty, &mut paused, has_vsync, &mut surface_system, &mut render_server) {
                                        LoopCtl::Continue => {}
                                        LoopCtl::Shutdown => {
                                            shared::stats::unregister_stats(host_id);
                                            return;
                                        }
                                    }
                                    // Drain remaining pending commands.
                                    match drain_cmds(&mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut frame_scheduler, &mut ticker, &mut dirty, &mut paused, &mut surface_system, &mut render_server) {
                                        LoopCtl::Continue => {}
                                        LoopCtl::Shutdown => {
                                            shared::stats::unregister_stats(host_id);
                                            return;
                                        }
                                    }
                                }
                                Err(_) => {
                                    info!("Command channel closed, exiting RenderThread");
                                    cm.destroy_all(&gl);
                                    shared::stats::unregister_stats(host_id);
                                    return;
                                }
                            }
                        }
                    }
                }
                })); // end catch_unwind
                if let Err(panic_info) = result {
                    let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                        s.clone()
                    } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                        s.to_string()
                    } else {
                        "Unknown panic".to_string()
                    };
                    error!("[RenderThread host={}] PANIC: {}", host_id, msg);
                    if let Some(stats) = shared::stats::get_stats(host_id) {
                        stats.fatal_error_code.store(
                            shared::error::ErrorCode::Internal.as_u16() as u32,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                    }
                    shared::stats::unregister_stats(host_id);
                }
            })
            .map_err(|e| {
                EngineError::from_detail(
                    ErrorCode::IoError,
                    format!("failed to spawn render thread: {}", e),
                )
            })?;

        Ok(Self {
            cmd_tx,
            handle: Some(handle),
        })
    }

    pub fn sender(&self) -> CommandSender {
        self.cmd_tx.clone()
    }

    pub fn shutdown(&mut self) {
        let _ = self.cmd_tx.send(RenderCommand::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }

    pub fn shutdown_detached(&mut self) {
        let _ = self.cmd_tx.send(RenderCommand::Shutdown);
        if let Some(h) = self.handle.take() {
            std::thread::spawn(move || {
                let _ = h.join();
            });
        }
    }
}
