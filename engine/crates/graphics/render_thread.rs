use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use crate::{onscreen_window_from_surface, CanvasHandler, CanvasManager, FontData, Renderer2d, RendererGL, global_fonts_mut};
use crossbeam_channel::{bounded, select, tick, Receiver};
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
        Self::spawn_with_capacity(raf_tx, vsync_rx, host_id, initial_surface, dpi, DEFAULT_RENDER_QUEUE_CAPACITY)
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
                #[cfg(target_os = "android")]
                const EGL_LIB: &str = "libEGL.so";
                #[cfg(not(target_os = "android"))]
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

                let mut canvas_handler = CanvasHandler::new();
                let mut renderer_2d = Renderer2d::new();
                let mut renderer_gl = RendererGL::new();

                // ---- FPS stats ----
                let debug_stats = shared::stats::register_stats(host_id);
                let mut frame_count: u32 = 0;
                let mut fps_timer = Instant::now();
                let mut last_frame_time = Instant::now();

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
                                          has_surface: &mut bool|
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
                                *frame_divisor = (60 / new_fps).max(1);
                                info!("RenderThread frame_divisor={} (target {}fps)", frame_divisor, new_fps);
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
                            // Track RecreateOnscreen to update has_surface.
                            let is_recreate = matches!(&canvas_cmd, shared::protocol::render_cmd::CanvasCmd::RecreateOnscreen { .. });
                            match canvas_handler.handle_command(cm, canvas_cmd) {
                                Ok(()) => {
                                    if is_recreate {
                                        *has_surface = true;
                                        info!("RenderThread surface recreated");
                                    }
                                    *dirty = true;
                                }
                                Err(e) => {
                                    error!("CanvasCmd failed: {}", e);
                                }
                            }
                        }

                        RenderCommand::GL(gl_cmd) => match renderer_gl.handle_command(cm, gl, gl_cmd) {
                            Ok(was_render) => {
                                if was_render {
                                    *dirty = true;
                                }
                            }
                            Err(e) => error!("GLCmd failed: {}", e),
                        },

                        RenderCommand::Canvas2D { canvas_id, cmd } => match renderer_2d.handle_command(cm, canvas_id, cmd) {
                            Ok(was_render) => {
                                if was_render {
                                    cm.mark_2d_dirty(canvas_id);
                                    *dirty = true;
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
                                      has_surface: &mut bool|
                 -> LoopCtl {
                    while let Ok(cmd) = cmd_rx.try_recv() {
                        match handle_one_cmd(cmd, cm, gl, canvas_handler, renderer_2d, renderer_gl, fps, ticker, dirty, paused, frame_divisor, has_vsync, has_surface) {
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
                                                        last_frame_time: &mut Instant| {
                    // Present the completed frame (only if we have a valid surface).
                    if *dirty && has_surface {
                        if let Err(e) = cm.flush_dirty_2d_contexts() {
                            warn!("flush_dirty_2d_contexts failed: {}", e);
                        }

                        if let Err(e) = cm.swap_buffers_no_restore(shared::protocol::render_cmd::CanvasId::from(1u32), true) {
                            warn!("swap_buffers_no_restore failed: {}", e);
                        }
                        *dirty = false;
                    }

                    // FPS stats update.
                    let now = Instant::now();
                    let frame_dur = now.duration_since(*last_frame_time);
                    *last_frame_time = now;
                    debug_stats.frame_time_us.store(frame_dur.as_micros() as u32, Ordering::Relaxed);

                    *frame_count += 1;
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
                    select! {
                        recv(ticker) -> _ => {
                            // Software ticker path (non-Android fallback).
                            // Frame timing: drain → swap → RAF signal.

                            // 1) Drain all pending commands from the previous frame.
                            match drain_cmds(&mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut ticker, &mut dirty, &mut paused, &mut frame_divisor, &mut has_surface) {
                                LoopCtl::Continue => {}
                                LoopCtl::Shutdown => {
                                    shared::stats::unregister_stats(host_id);
                                    return;
                                }
                            }

                            // 2) Present frame and signal RAF.
                            let ts = start_time.elapsed().as_secs_f64() * 1000.0;
                            present_frame_and_signal_raf(&mut cm, &mut dirty, has_surface, ts, &debug_stats, &mut frame_count, &mut fps_timer, &mut last_frame_time);
                        }

                        recv(vsync) -> _msg => {
                            // Choreographer VSync path (Android).
                            // Discard VSync while paused or surface is destroyed.
                            if paused || !has_surface {
                                continue;
                            }

                            // Frame divisor: skip frames to achieve target fps.
                            vsync_count += 1;
                            if vsync_count % frame_divisor != 0 {
                                continue;
                            }

                            // 1) Drain all pending commands.
                            match drain_cmds(&mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut ticker, &mut dirty, &mut paused, &mut frame_divisor, &mut has_surface) {
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
                            present_frame_and_signal_raf(&mut cm, &mut dirty, has_surface, ts, &debug_stats, &mut frame_count, &mut fps_timer, &mut last_frame_time);
                        }

                        recv(cmd_rx) -> msg => {
                            match msg {
                                Ok(cmd) => {
                                    match handle_one_cmd(cmd, &mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut ticker, &mut dirty, &mut paused, &mut frame_divisor, has_vsync, &mut has_surface) {
                                        LoopCtl::Continue => {}
                                        LoopCtl::Shutdown => {
                                            shared::stats::unregister_stats(host_id);
                                            return;
                                        }
                                    }

                                    match drain_cmds(&mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut ticker, &mut dirty, &mut paused, &mut frame_divisor, &mut has_surface) {
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
            })
            .unwrap();

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
