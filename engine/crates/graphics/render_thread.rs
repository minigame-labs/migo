use std::thread;
use std::time::{Duration, Instant};

use crate::{onscreen_window_from_surface, CanvasHandler, CanvasManager, Renderer2d, RendererGL};
use crossbeam_channel::{bounded, select, tick, Receiver};
use shared::protocol::host_cmd::HostCommand;
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
    pub fn spawn(js_tx: TokioSender<HostCommand>, initial_surface: Option<SurfaceRef>, dpi: f32) -> Self {
        Self::spawn_with_capacity(js_tx, initial_surface, dpi, DEFAULT_RENDER_QUEUE_CAPACITY)
    }

    /// Spawn render thread with custom queue capacity.
    pub fn spawn_with_capacity(
        js_tx: TokioSender<HostCommand>,
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

                // Render loop state.
                let mut fps: u32 = 60;
                let mut ticker: Receiver<Instant> = tick(Duration::from_secs_f32(1.0 / fps as f32));
                let start_time = Instant::now();
                let mut dirty = true;

                let mut canvas_handler = CanvasHandler::new();
                let mut renderer_2d = Renderer2d::new();
                let mut renderer_gl = RendererGL::new();

                // Stats.
                let mut dropped_raf = 0u64;

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
                                          dirty: &mut bool|
                 -> LoopCtl {
                    match cmd {
                        RenderCommand::Shutdown => {
                            info!("RenderThread received Shutdown");
                            cm.destroy_all(&gl);
                            return LoopCtl::Shutdown;
                        }

                        RenderCommand::FrameRate(new_fps) => {
                            let new_fps = new_fps.max(1);
                            if new_fps != *fps {
                                *fps = new_fps;
                                *ticker = tick(Duration::from_secs_f32(1.0 / *fps as f32));
                                info!("RenderThread fps changed to {}", fps);
                            }
                        }

                        RenderCommand::Canvas(canvas_cmd) => {
                            if let Err(e) = canvas_handler.handle_command(cm, canvas_cmd) {
                                error!("CanvasCmd failed: {}", e);
                            } else {
                                *dirty = true;
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
                                      dirty: &mut bool|
                 -> LoopCtl {
                    while let Ok(cmd) = cmd_rx.try_recv() {
                        match handle_one_cmd(cmd, cm, gl, canvas_handler, renderer_2d, renderer_gl, fps, ticker, dirty) {
                            LoopCtl::Continue => {}
                            LoopCtl::Shutdown => return LoopCtl::Shutdown,
                        }
                    }
                    LoopCtl::Continue
                };

                loop {
                    select! {
                        recv(ticker) -> _ => {
                            // 1) Send RAF to JS (non-blocking).
                            let ts = start_time.elapsed().as_secs_f64() * 1000.0;
                            if let Err(_e) = js_tx.try_send(HostCommand::RequestAnimationFrame(ts)) {
                                dropped_raf += 1;
                                if dropped_raf % 100 == 0 {
                                    warn!("dropped raf messages: {}", dropped_raf);
                                }
                            }

                            // 2) Drain commands before rendering to reduce latency.
                            match drain_cmds(&mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut ticker, &mut dirty) {
                                LoopCtl::Continue => {}
                                LoopCtl::Shutdown => return,
                            }

                            // 3) Present when dirty.
                            if dirty {
                                if let Err(e) = cm.flush_dirty_2d_contexts() {
                                    warn!("flush_dirty_2d_contexts failed: {}", e);
                                }

                            
                                if let Err(e) = cm.swap_buffers_no_restore(shared::protocol::render_cmd::CanvasId::from(1u32), true) {
                                    warn!("swap_buffers_no_restore failed: {}", e);
                                }
                                dirty = false;
                            }
                        }

                        recv(cmd_rx) -> msg => {
                            match msg {
                                Ok(cmd) => {
                                    match handle_one_cmd(cmd, &mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut ticker, &mut dirty) {
                                        LoopCtl::Continue => {}
                                        LoopCtl::Shutdown => return,
                                    }

                                    match drain_cmds(&mut cm, &gl, &mut canvas_handler, &mut renderer_2d, &mut renderer_gl, &mut fps, &mut ticker, &mut dirty) {
                                        LoopCtl::Continue => {}
                                        LoopCtl::Shutdown => return,
                                    }
                                }
                                Err(_) => {
                                    info!("cmd_rx closed, exiting RenderThread");
                                    cm.destroy_all(&gl);
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
