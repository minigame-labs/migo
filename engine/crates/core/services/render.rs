use std::time::Duration;

use crossbeam_channel::{RecvTimeoutError, bounded};
use tracing::{info, warn};

use graphics::{RenderThread, SurfaceSystem};

use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    protocol::render_cmd::{CanvasCmd, RenderCmdResp, RenderCommand},
    render_event::RenderEventReceiver,
    surface::SurfaceRef,
};

pub(crate) struct RenderService {
    surface: Option<SurfaceRef>,
    surface_system: SurfaceSystem,
    thread: RenderThread,
}

fn surface_for_restore(surface: Option<SurfaceRef>) -> EngineResult<SurfaceRef> {
    surface.ok_or_else(|| {
        EngineError::new(ErrorCode::InvalidOperation)
            .with_msg("restore surface: no live surface available")
    })
}

#[cfg(test)]
mod tests {
    use super::surface_for_restore;

    #[test]
    fn restore_surface_requires_live_surface() {
        let err = surface_for_restore(None).unwrap_err();
        assert_eq!(err.code, shared::error::ErrorCode::InvalidOperation);
        assert_eq!(err.msg, "restore surface: no live surface available");
    }
}

impl RenderService {
    pub(crate) const RECREATE_ONSCREEN_TIMEOUT: Duration = Duration::from_millis(500);

    pub(crate) fn new(
        raf_tx: shared::raf_signal::RafSender,
        vsync_rx: Option<crossbeam_channel::Receiver<f64>>,
        host_id: i32,
        surface: SurfaceRef,
        pixel_ratio: f32,
        target_fps: i32,
        app_cache_dir: Option<std::path::PathBuf>,
        gpu_caps: std::sync::Arc<shared::device::gpu_caps::GpuCaps>,
    ) -> EngineResult<Self> {
        let thread = RenderThread::spawn(
            raf_tx,
            vsync_rx,
            host_id,
            Some(surface.clone()),
            pixel_ratio,
            app_cache_dir,
            gpu_caps,
        )?;
        // Apply the host's configured target FPS to the render thread immediately
        // so the first vsync tick already runs at the right cadence.
        let _ = thread
            .sender()
            .send(RenderCommand::FrameRate(target_fps.clamp(1, 120) as u32));
        let mut surface_system = SurfaceSystem::new();
        surface_system.on_surface_available(surface.size());
        Ok(Self {
            surface: Some(surface),
            surface_system,
            thread,
        })
    }

    #[inline]
    pub(crate) fn sender(&self) -> shared::render_command_sender::CommandSender {
        self.thread.sender()
    }

    #[inline]
    pub(crate) fn events(&self) -> RenderEventReceiver {
        self.thread.events()
    }

    /// F-2: pass-through accessor so `HostOpState` can adopt the
    /// render thread's `SharedTextMeasurer`.  The measurer is
    /// built at `RenderThread::spawn` time and lives for the
    /// lifetime of the render service.
    #[inline]
    pub(crate) fn text_measurer(&self) -> shared::text_measurer::SharedTextMeasurer {
        self.thread.text_measurer()
    }

    /// Update onscreen surface and request backend recreate.
    pub(crate) fn update_surface(&mut self, surface: SurfaceRef) -> EngineResult<()> {
        let surface_size = surface.size();

        let (tx, rx) = bounded::<Result<(), EngineError>>(1);
        let cmd = RenderCommand::Canvas(CanvasCmd::RecreateOnscreen {
            surface: surface.clone(),
            resp: RenderCmdResp::from_sync(tx),
        });

        self.sender().send(cmd).map_err(|e| {
            EngineError::new(ErrorCode::Cancelled)
                .with_msg("recreate onscreen: send failed")
                .with_detail(e.to_string())
        })?;

        match rx.recv_timeout(Self::RECREATE_ONSCREEN_TIMEOUT) {
            Ok(Ok(())) => {
                self.surface = Some(surface);
                self.surface_system.on_surface_available(surface_size);
                info!(
                    "RenderService::update_surface ok: requested={}x{}",
                    surface_size.0, surface_size.1
                );
                Ok(())
            }

            Ok(Err(e)) => {
                warn!(
                    "RenderService::update_surface backend error: requested={}x{}, err={}",
                    surface_size.0, surface_size.1, e
                );
                Err(e)
            }

            Err(RecvTimeoutError::Timeout) => {
                warn!(
                    "RenderService::update_surface timeout: requested={}x{}, waited={}ms",
                    surface_size.0,
                    surface_size.1,
                    Self::RECREATE_ONSCREEN_TIMEOUT.as_millis()
                );
                Err(EngineError::new(ErrorCode::Timeout)
                    .with_msg("recreate onscreen: timed out")
                    .with_detail(format!(
                        "timed out after {}ms",
                        Self::RECREATE_ONSCREEN_TIMEOUT.as_millis()
                    )))
            }

            Err(e) => {
                warn!(
                    "RenderService::update_surface recv failed: requested={}x{}, err={:?}",
                    surface_size.0, surface_size.1, e
                );
                Err(EngineError::new(ErrorCode::Cancelled)
                    .with_msg("recreate onscreen: recv failed")
                    .with_detail(format!("{e:?}")))
            }
        }
    }

    /// Pause rendering (stop RAF ticker and frame presentation).
    pub(crate) fn pause(&mut self) {
        self.surface_system.on_pause();
        let _ = self.sender().send(RenderCommand::Pause);
    }

    /// Record surface loss and clear any stale surface handle.
    pub(crate) fn on_surface_destroyed(&mut self) {
        self.surface = None;
        self.surface_system.on_surface_destroyed();
        let _ = self.sender().send(RenderCommand::SurfaceDestroyed);
    }

    /// Resume rendering (restart RAF ticker and frame presentation).
    pub(crate) fn resume(&mut self) {
        self.surface_system.on_resume();
        let _ = self.sender().send(RenderCommand::Resume);
    }

    /// Re-signal the current live surface to the render thread.
    ///
    /// This is only valid if the session still retains a live `SurfaceRef`.
    /// After `on_surface_destroyed()`, the handle is cleared and callers must
    /// wait for a fresh `update_surface()` instead of reusing a stale surface.
    pub(crate) fn restore_surface(&mut self) -> EngineResult<()> {
        self.update_surface(surface_for_restore(self.surface.clone())?)
    }

    pub(crate) fn shutdown(&mut self) {
        self.thread.shutdown();
    }

    pub(crate) fn shutdown_detached(&mut self) {
        self.thread.shutdown_detached();
    }
}
