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

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        raf_tx: shared::raf_signal::RafSender,
        vsync_rx: Option<crossbeam_channel::Receiver<f64>>,
        host_id: i32,
        surface: SurfaceRef,
        pixel_ratio: f32,
        target_fps: i32,
        app_cache_dir: Option<std::path::PathBuf>,
        gpu_caps: std::sync::Arc<shared::device::gpu_caps::GpuCaps>,
        context_lost: std::sync::Arc<shared::op_state::ContextLostState>,
        wake: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
        raf_demand: shared::raf_signal::RafDemandRef,
        request_vsync: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    ) -> EngineResult<Self> {
        // Per-host surface destroy-epoch: bumped by JNI on surfaceDestroyed,
        // captured onto each new SurfaceRef at updateSurface time, and compared
        // by the render thread every frame so it stops presenting to a surface
        // that was torn down after hand-off (queue-independent, ABA-proof).
        // Init 0 — the initial surface (below) is stamped epoch 0 to match.
        let destroy_epoch = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        crate::runtime::registry::register_destroy_epoch(host_id, destroy_epoch.clone());

        let thread = RenderThread::spawn(
            raf_tx,
            vsync_rx,
            host_id,
            Some(surface.clone()),
            pixel_ratio,
            app_cache_dir,
            gpu_caps,
            context_lost,
            wake,
            raf_demand,
            request_vsync,
            destroy_epoch,
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

    /// Whether the host currently holds a live onscreen surface. False after
    /// `on_surface_destroyed()` until the next successful `update_surface()`.
    #[inline]
    pub(crate) fn has_live_surface(&self) -> bool {
        self.surface.is_some()
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

        // RecreateOnscreen carries a sync responder; route it through the
        // policy-aware `dispatch` (bounded-blocking for its Sync class) rather
        // than the legacy drop-on-full `send`, so a transiently full render queue
        // doesn't silently drop the recreate and strand the reply/onShow.
        self.sender().dispatch(cmd).map_err(|e| {
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
        // Bounded-blocking: dropping these on a full render queue desyncs the
        // render thread from the surface/lifecycle state (a dropped Resume
        // leaves the app frozen; a dropped SurfaceDestroyed leaves the render
        // thread presenting to a dead surface), so wait rather than drop.
        let _ = self.sender().send_blocking_bounded(RenderCommand::Pause);
    }

    /// Record surface loss and clear any stale surface handle.
    pub(crate) fn on_surface_destroyed(&mut self) {
        self.surface = None;
        self.surface_system.on_surface_destroyed();
        let _ = self
            .sender()
            .send_blocking_bounded(RenderCommand::SurfaceDestroyed);
    }

    /// Resume rendering (restart RAF ticker and frame presentation).
    pub(crate) fn resume(&mut self) {
        self.surface_system.on_resume();
        let _ = self.sender().send_blocking_bounded(RenderCommand::Resume);
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
