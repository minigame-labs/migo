use std::time::Duration;

use crossbeam_channel::{RecvTimeoutError, bounded};
use tracing::{info, warn};

use graphics::RenderThread;

use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    protocol::render_cmd::{CanvasCmd, RenderCmdResp, RenderCommand},
    surface::SurfaceRef,
};

pub(crate) struct RenderService {
    surface: SurfaceRef,
    thread: RenderThread,
}

impl RenderService {
    pub(crate) const RECREATE_ONSCREEN_TIMEOUT: Duration = Duration::from_millis(500);

    pub(crate) fn new(
        raf_tx: tokio::sync::mpsc::Sender<f64>,
        vsync_rx: Option<crossbeam_channel::Receiver<f64>>,
        host_id: i32,
        surface: SurfaceRef,
        pixel_ratio: f32,
    ) -> Self {
        let thread = RenderThread::spawn(
            raf_tx,
            vsync_rx,
            host_id,
            Some(surface.clone()),
            pixel_ratio,
        );
        Self { surface, thread }
    }

    #[inline]
    pub(crate) fn sender(&self) -> crossbeam_channel::Sender<RenderCommand> {
        self.thread.sender()
    }

    /// Update onscreen surface and request backend recreate.
    pub(crate) fn update_surface(&mut self, surface: SurfaceRef) -> EngineResult<()> {
        let surface_size = surface.size();
        info!(
            "RenderService::update_surface begin: requested={}x{}",
            surface_size.0, surface_size.1
        );
        self.surface = surface.clone();

        let (tx, rx) = bounded::<Result<(), EngineError>>(1);
        let cmd = RenderCommand::Canvas(CanvasCmd::RecreateOnscreen {
            surface,
            resp: RenderCmdResp::Sync(tx),
        });

        self.sender().send(cmd).map_err(|e| {
            EngineError::new(ErrorCode::Cancelled)
                .with_msg("recreate onscreen: send failed")
                .with_detail(e.to_string())
        })?;

        match rx.recv_timeout(Self::RECREATE_ONSCREEN_TIMEOUT) {
            Ok(Ok(())) => {
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
    pub(crate) fn pause(&self) {
        let _ = self.sender().send(RenderCommand::Pause);
    }

    /// Resume rendering (restart RAF ticker and frame presentation).
    pub(crate) fn resume(&self) {
        let _ = self.sender().send(RenderCommand::Resume);
    }

    /// Re-signal the current surface to the render thread.
    ///
    /// During `Pause`, the render thread sets `has_surface = false`.
    /// In the normal OnHide→UpdateSurface flow, `RecreateOnscreen` restores it.
    /// For restart (where the surface hasn't changed), call this before `resume()`
    /// to restore `has_surface = true` so VSync frames are no longer discarded.
    ///
    /// This delegates to [`update_surface`](Self::update_surface), which sends a
    /// `RecreateOnscreen` command through the crossbeam channel and blocks until
    /// the render thread acknowledges (with a timeout). The command channel
    /// provides the cross-thread synchronization: the caller will not return
    /// until the render thread has processed the command and set `has_surface`.
    pub(crate) fn restore_surface(&mut self) -> EngineResult<()> {
        self.update_surface(self.surface.clone())
    }

    pub(crate) fn shutdown(&mut self) {
        self.thread.shutdown();
    }
}
