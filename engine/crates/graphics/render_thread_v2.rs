//! # Render Thread V2
//!
//! Improved render thread with:
//! - Command batching (one message per frame)
//! - Multiple render modes (continuous, on-demand)
//! - Better frame pacing
//! - Cross-platform backend abstraction
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                           JS Thread                                  │
//! │                                                                      │
//! │   RAF → CommandEncoder.fill_rect() → .draw_image() → .finish()      │
//! │                                            │                         │
//! │                                            ▼                         │
//! │                                    CommandBuffer                     │
//! │                                            │                         │
//! └────────────────────────────────────────────┼─────────────────────────┘
//!                                              │
//!                            FrameSubmitter.submit(buffer)
//!                                              │
//!                                              ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                        Render Thread V2                              │
//! │                                                                      │
//! │   ┌──────────────┐    ┌──────────────┐    ┌──────────────┐          │
//! │   │    Frame     │    │   Command    │    │   Backend    │          │
//! │   │  Scheduler   │───▶│   Executor   │───▶│  (EGL/GL)    │          │
//! │   └──────────────┘    └──────────────┘    └──────────────┘          │
//! │                                                  │                   │
//! │                                                  ▼                   │
//! │                                           swap_buffers              │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```

use std::thread;
use std::time::Instant;

use crossbeam_channel::Receiver;
use tracing::{error, info, warn};

use crate::backend::{EglBackend, RenderBackend, SurfaceConfig, SurfaceId};
use crate::command_buffer::{Canvas2DCommand, CommandBuffer};
use crate::scheduler::{
    FrameConfig, FrameScheduler, FrameStats, RenderMode, SchedulerMessage,
    create_scheduler_channel, FrameSubmitter,
};
use crate::{Canvas2DContext, FontManager};

use femtovg::{Canvas as FvCanvas, renderer::OpenGl};
use shared::protocol::host_cmd::HostCommand;
use tokio::sync::mpsc::Sender as TokioSender;

/// Render thread V2 with improved architecture
pub struct RenderThreadV2 {
    /// Frame submitter for sending command buffers
    submitter: FrameSubmitter,
    
    /// Thread handle
    handle: Option<thread::JoinHandle<()>>,
}

impl RenderThreadV2 {
    /// Spawn a new render thread with the given configuration
    pub fn spawn(
        js_tx: TokioSender<HostCommand>,
        initial_window: Option<usize>,
        dpi: f32,
        config: FrameConfig,
    ) -> Self {
        let (submitter, rx) = create_scheduler_channel(4);
        
        let handle = thread::Builder::new()
            .name("Migo-RenderThread-V2".into())
            .spawn(move || {
                if let Err(e) = run_render_loop(rx, js_tx, initial_window, dpi, config) {
                    error!("Render thread error: {}", e);
                }
            })
            .expect("failed to spawn render thread");
        
        Self {
            submitter,
            handle: Some(handle),
        }
    }
    
    /// Get a frame submitter for sending command buffers
    pub fn submitter(&self) -> FrameSubmitter {
        self.submitter.clone()
    }
    
    /// Configure rendering mode
    pub fn configure(&self, config: FrameConfig) {
        let _ = self.submitter.configure(config);
    }
    
    /// Request a redraw (for on-demand mode)
    pub fn invalidate(&self) {
        self.submitter.invalidate();
    }
    
    /// Shutdown the render thread
    pub fn shutdown(&mut self) {
        let _ = self.submitter.shutdown();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for RenderThreadV2 {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Main render loop
fn run_render_loop(
    rx: Receiver<SchedulerMessage>,
    js_tx: TokioSender<HostCommand>,
    initial_window: Option<usize>,
    dpi: f32,
    initial_config: FrameConfig,
) -> Result<(), String> {
    // Initialize backend
    #[cfg(target_os = "android")]
    const EGL_LIB: &str = "libEGL.so";
    #[cfg(not(target_os = "android"))]
    const EGL_LIB: &str = "libEGL.so";
    
    let mut backend = EglBackend::new(EGL_LIB)
        .map_err(|e| format!("backend init failed: {}", e))?;
    
    // Create onscreen surface if window provided
    let onscreen_surface: Option<SurfaceId> = if let Some(window) = initial_window {
        let config = SurfaceConfig::default();
        Some(backend.create_onscreen_surface(window, &config)
            .map_err(|e| format!("create surface failed: {}", e))?)
    } else {
        None
    };
    
    // Initialize FemtoVG renderer
    let gl = unsafe {
        glow::Context::from_loader_function(|s| backend.get_proc_address(s))
    };
    
    let fv_renderer = unsafe {
        OpenGl::new_from_function(|s| backend.get_proc_address(s))
    }.map_err(|e| format!("FemtoVG init failed: {:?}", e))?;
    
    let mut fv_canvas = FvCanvas::new(fv_renderer)
        .map_err(|e| format!("FvCanvas init failed: {:?}", e))?;
    
    if let Some(surf_id) = onscreen_surface {
        backend.make_current(surf_id).ok();
        let (w, h) = backend.get_surface_size(surf_id).unwrap_or((800, 600));
        fv_canvas.set_size(w, h, dpi);
    }
    
    // Initialize font manager
    let font_manager = FontManager::new(&mut fv_canvas)
        .map_err(|e| format!("FontManager init failed: {}", e))?;
    
    // Create 2D context
    let mut ctx_2d = Canvas2DContext::new(fv_canvas, font_manager);
    
    // Frame scheduler
    let mut scheduler = FrameScheduler::new(initial_config);
    
    // Frame timing
    let start_time = Instant::now();
    
    info!("RenderThread V2 started, mode={:?}", scheduler.mode());
    
    // Main loop
    loop {
        // In continuous mode, trigger RAF based on scheduler timing
        if scheduler.mode() == RenderMode::Continuous {
            if scheduler.wait_for_frame_start() {
                let ts = start_time.elapsed().as_secs_f64() * 1000.0;
                let _ = js_tx.try_send(HostCommand::RequestAnimationFrame(ts));
            }
        }
        
        // Wait for command buffer
        let msg = match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(msg) => msg,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                // In on-demand mode, just wait
                continue;
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                info!("Render channel closed, shutting down");
                break;
            }
        };
        
        match msg {
            SchedulerMessage::Frame(buffer) => {
                let frame_start = Instant::now();
                
                // Execute commands
                let draw_count = execute_command_buffer(
                    &buffer,
                    &mut ctx_2d,
                    &mut backend,
                    onscreen_surface,
                );
                
                // Flush and present
                if let Some(surf_id) = onscreen_surface {
                    ctx_2d.flush();
                    backend.swap_buffers(surf_id, scheduler.mode() != RenderMode::Background).ok();
                }
                
                // Record stats
                let total_time = frame_start.elapsed();
                let stats = FrameStats {
                    frame_number: buffer.frame_id,
                    execute_time_us: total_time.as_micros() as u64,
                    total_time_us: total_time.as_micros() as u64,
                    command_count: buffer.command_count() as u32,
                    draw_call_count: draw_count,
                    ..Default::default()
                };
                scheduler.end_frame(stats);
            }
            
            SchedulerMessage::Invalidate => {
                scheduler.invalidate();
                
                // In on-demand mode, trigger RAF after invalidate
                if scheduler.mode() == RenderMode::OnDemand {
                    let ts = start_time.elapsed().as_secs_f64() * 1000.0;
                    let _ = js_tx.try_send(HostCommand::RequestAnimationFrame(ts));
                }
            }
            
            SchedulerMessage::Configure(config) => {
                scheduler.configure(config);
            }
            
            SchedulerMessage::Pause => {
                scheduler.configure(FrameConfig {
                    mode: RenderMode::Paused,
                    ..FrameConfig::default()
                });
            }
            
            SchedulerMessage::Resume => {
                scheduler.configure(FrameConfig::game());
                // Trigger immediate RAF on resume
                let ts = start_time.elapsed().as_secs_f64() * 1000.0;
                let _ = js_tx.try_send(HostCommand::RequestAnimationFrame(ts));
            }
            
            SchedulerMessage::Shutdown => {
                info!("Render thread received shutdown");
                break;
            }
        }
    }
    
    // Cleanup
    if let Some(surf_id) = onscreen_surface {
        backend.destroy_surface(surf_id).ok();
    }
    
    info!("RenderThread V2 stopped");
    Ok(())
}

/// Execute a command buffer and return draw call count
fn execute_command_buffer(
    buffer: &CommandBuffer,
    ctx: &mut Canvas2DContext,
    _backend: &mut EglBackend,
    _surface: Option<SurfaceId>,
) -> u32 {
    let mut draw_calls = 0u32;
    
    for cmd in &buffer.canvas_2d_commands {
        match cmd {
            // State
            Canvas2DCommand::Save => ctx.save(),
            Canvas2DCommand::Restore => ctx.restore(),
            
            // Styles
            Canvas2DCommand::SetFillColor(color) => ctx.set_fill_style_color(*color),
            Canvas2DCommand::SetStrokeColor(color) => ctx.set_stroke_style_color(*color),
            Canvas2DCommand::SetFillGradient { .. } => {
                // TODO: implement gradient
            }
            Canvas2DCommand::SetLineWidth(w) => ctx.set_line_width(*w),
            Canvas2DCommand::SetLineCap(cap) => ctx.set_line_cap(*cap),
            Canvas2DCommand::SetLineJoin(join) => ctx.set_line_join(*join),
            Canvas2DCommand::SetMiterLimit(limit) => ctx.set_miter_limit(*limit),
            Canvas2DCommand::SetGlobalAlpha(alpha) => ctx.set_global_alpha(*alpha),
            Canvas2DCommand::SetFont { .. } => {
                // TODO: implement font setting
            }
            Canvas2DCommand::SetTextAlign(_) => {}
            Canvas2DCommand::SetTextBaseline(_) => {}
            
            // Transform
            Canvas2DCommand::SetTransform { a, b, c, d, e, f } => {
                ctx.set_transform(*a, *b, *c, *d, *e, *f);
            }
            Canvas2DCommand::ResetTransform => ctx.reset_transform(),
            Canvas2DCommand::Translate { x, y } => ctx.translate(*x, *y),
            Canvas2DCommand::Rotate(angle) => ctx.rotate(*angle),
            Canvas2DCommand::Scale { x, y } => ctx.scale(*x, *y),
            
            // Path
            Canvas2DCommand::BeginPath => ctx.begin_path(),
            Canvas2DCommand::ClosePath => ctx.close_path(),
            Canvas2DCommand::MoveTo { x, y } => ctx.move_to(*x, *y),
            Canvas2DCommand::LineTo { x, y } => ctx.line_to(*x, *y),
            Canvas2DCommand::QuadraticCurveTo { cpx, cpy, x, y } => {
                ctx.quadratic_curve_to(*cpx, *cpy, *x, *y);
            }
            Canvas2DCommand::BezierCurveTo { cp1x, cp1y, cp2x, cp2y, x, y } => {
                ctx.bezier_curve_to(*cp1x, *cp1y, *cp2x, *cp2y, *x, *y);
            }
            Canvas2DCommand::Arc { x, y, radius, start_angle, end_angle, ccw } => {
                ctx.arc(*x, *y, *radius, *start_angle, *end_angle, *ccw);
            }
            Canvas2DCommand::ArcTo { x1, y1, x2, y2, radius } => {
                ctx.arc_to(*x1, *y1, *x2, *y2, *radius);
            }
            Canvas2DCommand::Rect { x, y, w, h } => ctx.rect(*x, *y, *w, *h),
            Canvas2DCommand::Ellipse { x, y, rx, ry, rotation, start, end, ccw } => {
                ctx.ellipse(*x, *y, *rx, *ry, *rotation, *start, *end, *ccw);
            }
            
            // Drawing
            Canvas2DCommand::Fill => {
                ctx.fill();
                draw_calls += 1;
            }
            Canvas2DCommand::Stroke => {
                ctx.stroke();
                draw_calls += 1;
            }
            Canvas2DCommand::Clip => ctx.clip(),
            Canvas2DCommand::FillRect { x, y, w, h } => {
                ctx.fill_rect(*x, *y, *w, *h);
                draw_calls += 1;
            }
            Canvas2DCommand::StrokeRect { x, y, w, h } => {
                ctx.stroke_rect(*x, *y, *w, *h);
                draw_calls += 1;
            }
            Canvas2DCommand::ClearRect { x, y, w, h } => {
                ctx.clear_rect(*x, *y, *w, *h);
                draw_calls += 1;
            }
            Canvas2DCommand::FillText { text, x, y, max_width } => {
                ctx.fill_text(text, *x, *y, max_width.unwrap_or(f32::MAX));
                draw_calls += 1;
            }
            Canvas2DCommand::StrokeText { text, x, y, max_width } => {
                ctx.stroke_text(text, *x, *y, max_width.unwrap_or(f32::MAX));
                draw_calls += 1;
            }
            Canvas2DCommand::DrawImage { .. } => {
                // TODO: implement image drawing
                draw_calls += 1;
            }
            Canvas2DCommand::DrawImageBatch(images) => {
                // TODO: implement batch image drawing
                draw_calls += images.len() as u32;
            }
        }
    }
    
    draw_calls
}
