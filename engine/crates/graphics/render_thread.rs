use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use crate::{
    CanvasHandler, CanvasManager, FontData, Renderer2d, RendererGL, global_fonts_mut,
    onscreen_window_from_surface,
};
use crossbeam_channel::{Receiver, bounded, select, tick};
use shared::protocol::render_cmd::RenderCommand;
use shared::surface::SurfaceRef;
use tokio::sync::mpsc::Sender as TokioSender;
use tracing::{error, info, warn};

/// Default render command queue capacity.
/// Higher capacity reduces the chance of dropped frames under heavy load,
/// but uses more memory. 512 provides good balance for most games.
const DEFAULT_RENDER_QUEUE_CAPACITY: usize = 512;

pub struct RenderThread {
    cmd_tx: crossbeam_channel::Sender<RenderCommand>,
    handle: Option<thread::JoinHandle<()>>,
}

impl RenderThread {
    /// Spawn render thread with default queue capacity.
    ///
    /// * `raf_tx` — tokio mpsc sender for frame timestamps (render → JS async op)
    /// * `vsync_rx` — optional crossbeam receiver for Choreographer VSync signals
    /// * `host_id` — host identifier for debug stats registry
    pub fn spawn(
        raf_tx: TokioSender<f64>,
        vsync_rx: Option<Receiver<f64>>,
        host_id: i32,
        initial_surface: Option<SurfaceRef>,
        dpi: f32,
    ) -> Self {
        Self::spawn_with_capacity(
            raf_tx,
            vsync_rx,
            host_id,
            initial_surface,
            dpi,
            DEFAULT_RENDER_QUEUE_CAPACITY,
        )
    }

    /// Spawn render thread with custom queue capacity.
    pub fn spawn_with_capacity(
        raf_tx: TokioSender<f64>,
        vsync_rx: Option<Receiver<f64>>,
        host_id: i32,
        initial_surface: Option<SurfaceRef>,
        dpi: f32,
        queue_capacity: usize,
    ) -> Self {
        let (cmd_tx, cmd_rx) = bounded::<RenderCommand>(queue_capacity);

        let handle = std::thread::Builder::new()
            .name("Migo-RenderThread".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // Both Android and non-Android use the same EGL library name.
                const EGL_LIB: &str = "libEGL.so";

                let mut cm = match CanvasManager::new_with_resource(EGL_LIB, dpi) {
                    Ok(c) => c,
                    Err(e) => {
                        error!("CanvasManager init failed: {}", e);
                        return;
                    }
                };

                let has_initial_surface = initial_surface.is_some();

                // If an initial surface is provided, create the onscreen context immediately.
                if let Some(surface) = initial_surface {
                    match onscreen_window_from_surface(surface.as_ref()) {
                        Ok(win) => {
                            if let Err(e) = cm.create_onscreen(win) {
                                error!("create_onscreen failed: {}", e);
                            }
                        }
                        Err(e) => error!("initial surface is not supported: {}", e),
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

                let start_time = Instant::now();
                let mut dirty = true;
                let mut paused = false;
                // Track whether we have a valid EGL surface for swap_buffers.
                // Set to false on Pause (surface may be destroyed while backgrounded),
                // restored to true when RecreateOnscreen succeeds.
                let mut has_surface = has_initial_surface;

                // Frame divisor for Choreographer fps control.
                // E.g., frame_divisor=2 means deliver every 2nd VSync → ~30fps on 60Hz.
                let mut frame_divisor: u32 = 1;
                let mut vsync_count: u32 = 0;
                // Auto-detected VSync frequency (updated from timestamps).
                // Defaults to 60Hz; measured over 1-second windows to handle
                // 90Hz, 120Hz, and variable-rate displays correctly.
                let mut detected_vsync_hz: u32 = 60;
                let mut vsync_hz_counter: u32 = 0;
                let mut vsync_hz_timer = Instant::now();
                // User-requested target FPS (used to recompute divisor on Hz change).
                let mut target_fps: u32 = 60;

                let mut canvas_handler = CanvasHandler::new();
                let mut renderer_2d = Renderer2d::new();
                let mut renderer_gl = RendererGL::new();

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
                                          ticker: &mut Receiver<Instant>,
                                          dirty: &mut bool,
                                          paused: &mut bool,
                                          frame_divisor: &mut u32,
                                          has_vsync: bool,
                                          has_surface: &mut bool,
                                          detected_vsync_hz: u32,
                                          target_fps: &mut u32|
                 -> LoopCtl {
                    match cmd {
                        RenderCommand::Shutdown => {
                            info!("RenderThread received Shutdown");
                            cm.destroy_all(&gl);
                            return LoopCtl::Shutdown;
                        }

                        RenderCommand::FrameRate(new_fps) => {
                            let new_fps = new_fps.clamp(1, 60);
                            if has_vsync {
                                // Choreographer mode: skip VSync signals to achieve target fps.
                                // Use auto-detected VSync frequency (not hardcoded 60Hz) to
                                // handle 90/120Hz displays correctly.
                                *frame_divisor = (detected_vsync_hz / new_fps).max(1);
                                *target_fps = new_fps;
                                info!("RenderThread frame_divisor={} (target {}fps, vsync {}Hz)", frame_divisor, new_fps, detected_vsync_hz);
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
                            let affects_onscreen = match &canvas_cmd {
                                shared::protocol::render_cmd::CanvasCmd::RecreateOnscreen { .. } => true,
                                shared::protocol::render_cmd::CanvasCmd::ResizeCanvas { id, .. } => *id == 1,
                                _ => false,
                            };
                            match canvas_handler.handle_command(cm, canvas_cmd) {
                                Ok(()) => {
                                    if is_recreate {
                                        *has_surface = true;
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
                            Ok(was_render) => {
                                // Only trigger onscreen present if GL targets the onscreen canvas
                                if was_render && cm.is_onscreen_bound() {
                                    *dirty = true;
                                }
                            }
                            Err(e) => error!("GLCmd failed: {}", e),
                        },
                        RenderCommand::GLBatch { commands } => {
                            let cmd_count = commands.len();
                            let mut batch_hit_onscreen = false;
                            let mut error_count: u32 = 0;
                            for gl_cmd in commands {
                                match renderer_gl.handle_command(cm, gl, gl_cmd) {
                                    Ok(was_render) => {
                                        // Check is_onscreen_bound per-command so we don't
                                        // miss onscreen renders when the batch also targets
                                        // offscreen canvases.
                                        if was_render && cm.is_onscreen_bound() {
                                            batch_hit_onscreen = true;
                                        }
                                    }
                                    Err(e) => {
                                        // Log first error in detail; suppress the rest to avoid log spam.
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
                            if batch_hit_onscreen {
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
                        RenderCommand::Canvas2DBatch { canvas_id, commands } => {
                            let mut batch_dirty = false;
                            for cmd in commands {
                                match renderer_2d.handle_command(cm, canvas_id, cmd) {
                                    Ok(was_render) => {
                                        if was_render {
                                            batch_dirty = true;
                                        }
                                    }
                                    Err(e) => error!("Canvas2DBatch cmd failed: {}", e),
                                }
                            }
                            if batch_dirty {
                                cm.mark_2d_dirty(canvas_id);
                                // Set dirty for onscreen canvas so the next VSync presents.
                                // Safe because mid-frame flushes (measureText, getImageData)
                                // no longer send Canvas2DBatch — only frame-end does.
                                if canvas_id == 1 {
                                    *dirty = true;
                                }
                            }
                        }

                        // Invalidate signal for on-demand rendering
                        RenderCommand::Invalidate => {
                            *dirty = true;
                        }

                        RenderCommand::Pause => {
                            if !*paused {
                                *paused = true;
                                // Mark surface as unavailable — the Android surface may be
                                // destroyed while backgrounded. This prevents any lingering
                                // VSync frames from attempting swap_buffers on a dead surface.
                                *has_surface = false;
                                if !has_vsync {
                                    *ticker = crossbeam_channel::never();
                                }
                                info!("RenderThread paused");
                            }
                        }

                        RenderCommand::Resume => {
                            if *paused {
                                *paused = false;
                                if !has_vsync {
                                    *ticker = tick(Duration::from_secs_f32(1.0 / *fps as f32));
                                }
                                info!("RenderThread resumed");
                            }
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
                                      ticker: &mut Receiver<Instant>,
                                      dirty: &mut bool,
                                      paused: &mut bool,
                                      frame_divisor: &mut u32,
                                      has_surface: &mut bool,
                                      detected_vsync_hz: u32,
                                      target_fps: &mut u32|
                 -> LoopCtl {
                    while let Ok(cmd) = cmd_rx.try_recv() {
                        match handle_one_cmd(cmd, cm, gl, canvas_handler, renderer_2d, renderer_gl, fps, ticker, dirty, paused, frame_divisor, has_vsync, has_surface, detected_vsync_hz, target_fps) {
                            LoopCtl::Continue => {}
                            LoopCtl::Shutdown => return LoopCtl::Shutdown,
                        }
                    }
                    LoopCtl::Continue
                };

                // Shared frame presentation + RAF signal logic.
                let present_frame_and_signal_raf = |cm: &mut CanvasManager,
                                                        dirty: &mut bool,
                                                        has_surface: bool,
                                                        ts: f64,
                                                        debug_stats: &shared::stats::DebugStats,
                                                        frame_count: &mut u32,
                                                        fps_timer: &mut Instant,
                                                        last_frame_time: &mut Instant,
                                                        first_frame_recorded: &mut bool,
                                                        needs_recovery: &mut bool| {
                    // Present the completed frame (only if we have a valid surface).
                    let did_swap = if *dirty && has_surface {
                        if let Err(e) = cm.flush_dirty_2d_contexts() {
                            warn!("flush_dirty_2d_contexts failed: {}", e);
                        }

                        let swap_ok = match cm.swap_buffers_no_restore(shared::protocol::render_cmd::CanvasId::from(1u32), true) {
                            Ok(()) => true,
                            Err(e) => {
                                warn!("swap_buffers_no_restore failed: {}", e);
                                // On EGL context loss, defer recovery to the top of the
                                // next frame iteration rather than recovering inline.
                                // Rationale: try_recover_context() tears down and
                                // recreates the entire EGL surface+context, which is
                                // expensive and would delay the RAF signal in the
                                // time-critical swap path, risking cascading frame drops.
                                if cm.is_context_lost() {
                                    *needs_recovery = true;
                                    warn!("EGL context lost, recovery deferred to next frame");
                                }
                                false
                            }
                        };
                        *dirty = false;

                        // Record first frame time only after a successful swap_buffers.
                        if swap_ok && !*first_frame_recorded {
                            *first_frame_recorded = true;
                            let first_ms = start_time.elapsed().as_millis() as u32;
                            debug_stats.first_frame_ms.store(first_ms, Ordering::Relaxed);
                        }
                        swap_ok
                    } else {
                        false
                    };

                    // FPS stats: only count frames where swap_buffers was actually
                    // called so the metric reflects real GPU work, not tick rate.
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

                    // Send RAF timestamp to JS async op (non-blocking).
                    if let Err(_e) = raf_tx.try_send(ts) {
                        debug_stats.dropped_frames.fetch_add(1, Ordering::Relaxed);
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

                            // 1) Drain all pending commands from the previous frame.
                            match drain_cmds(&mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut ticker, &mut dirty, &mut paused, &mut frame_divisor, &mut has_surface, detected_vsync_hz, &mut target_fps) {
                                LoopCtl::Continue => {}
                                LoopCtl::Shutdown => {
                                    shared::stats::unregister_stats(host_id);
                                    return;
                                }
                            }

                            // 2) Present frame and signal RAF.
                            let ts = start_time.elapsed().as_secs_f64() * 1000.0;
                            present_frame_and_signal_raf(&mut cm, &mut dirty, has_surface, ts, &debug_stats, &mut frame_count, &mut fps_timer, &mut last_frame_time, &mut first_frame_recorded, &mut needs_context_recovery);
                        }

                        recv(vsync) -> _msg => {
                            // Choreographer VSync path (Android).
                            // Discard VSync while paused or surface is destroyed.
                            if paused || !has_surface {
                                continue;
                            }

                            // Auto-detect VSync frequency from signal count per second.
                            // This correctly handles 60/90/120Hz and variable-rate displays.
                            // Optimization: only call Instant::elapsed() every 60 VSync
                            // signals (~1s at 60Hz) to avoid the syscall overhead on every
                            // frame. The counter still increments every VSync for accuracy.
                            vsync_hz_counter += 1;
                            if vsync_hz_counter >= 60 {
                                let hz_elapsed = vsync_hz_timer.elapsed();
                                if hz_elapsed >= Duration::from_secs(1) {
                                    let measured = (vsync_hz_counter as f64 / hz_elapsed.as_secs_f64()).round() as u32;
                                    let new_hz = measured.clamp(30, 240);
                                    if new_hz != detected_vsync_hz {
                                        detected_vsync_hz = new_hz;
                                        // Recompute frame divisor for the updated rate.
                                        frame_divisor = (detected_vsync_hz / target_fps).max(1);
                                        info!("RenderThread: detected VSync {}Hz, frame_divisor={}", detected_vsync_hz, frame_divisor);
                                    }
                                    vsync_hz_counter = 0;
                                    vsync_hz_timer = Instant::now();
                                }
                            }

                            // Frame divisor: skip frames to achieve target fps.
                            vsync_count += 1;
                            if vsync_count % frame_divisor != 0 {
                                continue;
                            }

                            // 1) Drain all pending commands.
                            match drain_cmds(&mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut ticker, &mut dirty, &mut paused, &mut frame_divisor, &mut has_surface, detected_vsync_hz, &mut target_fps) {
                                LoopCtl::Continue => {}
                                LoopCtl::Shutdown => {
                                    shared::stats::unregister_stats(host_id);
                                    return;
                                }
                            }

                            // 2) Present frame and signal RAF.
                            // Always use relative timestamp (elapsed since render thread start)
                            // instead of the Choreographer's absolute frameTimeNanos. The RAF
                            // callback timestamp must be relative to the time origin — an
                            // absolute hardware timestamp causes broken animation calculations
                            // (huge first-frame delta, incorrect absolute positions).
                            let ts = start_time.elapsed().as_secs_f64() * 1000.0;
                            present_frame_and_signal_raf(&mut cm, &mut dirty, has_surface, ts, &debug_stats, &mut frame_count, &mut fps_timer, &mut last_frame_time, &mut first_frame_recorded, &mut needs_context_recovery);
                        }

                        recv(cmd_rx) -> msg => {
                            match msg {
                                Ok(cmd) => {
                                    match handle_one_cmd(cmd, &mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut ticker, &mut dirty, &mut paused, &mut frame_divisor, has_vsync, &mut has_surface, detected_vsync_hz, &mut target_fps) {
                                        LoopCtl::Continue => {}
                                        LoopCtl::Shutdown => {
                                            shared::stats::unregister_stats(host_id);
                                            return;
                                        }
                                    }

                                    match drain_cmds(&mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut ticker, &mut dirty, &mut paused, &mut frame_divisor, &mut has_surface, detected_vsync_hz, &mut target_fps) {
                                        LoopCtl::Continue => {}
                                        LoopCtl::Shutdown => {
                                            shared::stats::unregister_stats(host_id);
                                            return;
                                        }
                                    }
                                }
                                Err(_) => {
                                    info!("cmd_rx closed, exiting RenderThread");
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
            .expect("failed to spawn Migo-RenderThread");

        Self {
            cmd_tx,
            handle: Some(handle),
        }
    }

    pub fn sender(&self) -> crossbeam_channel::Sender<RenderCommand> {
        self.cmd_tx.clone()
    }

    pub fn shutdown(&mut self) {
        let _ = self.cmd_tx.send(RenderCommand::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
