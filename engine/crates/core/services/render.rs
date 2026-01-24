use std::time::Duration;

use crossbeam_channel::{RecvTimeoutError, bounded};

use graphics::RenderThread;

use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    protocol::{
        host_cmd::HostCommand,
        render_cmd::{CanvasCmd, RenderCmdResp, RenderCommand},
    },
    surface::SurfaceRef,
};

pub(crate) struct RenderService {
    surface: SurfaceRef,
    thread: RenderThread,
}

impl RenderService {
    pub(crate) const RECREATE_ONSCREEN_TIMEOUT: Duration = Duration::from_millis(500);

    pub(crate) fn new(
        js_tx: tokio::sync::mpsc::Sender<HostCommand>,
        surface: SurfaceRef,
        pixel_ratio: f32,
    ) -> Self {
        let thread = RenderThread::spawn(js_tx, Some(surface.clone()), pixel_ratio);
        Self { surface, thread }
    }

    #[inline]
    pub(crate) fn sender(&self) -> crossbeam_channel::Sender<RenderCommand> {
        self.thread.sender()
    }

    /// Update onscreen surface and request backend recreate.
    pub(crate) fn update_surface(&mut self, surface: SurfaceRef) -> EngineResult<()> {
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
            Ok(Ok(())) => Ok(()),

            Ok(Err(e)) => Err(e),

            Err(RecvTimeoutError::Timeout) => Err(EngineError::new(ErrorCode::Timeout)
                .with_msg("recreate onscreen: timed out")
                .with_detail(format!(
                    "timed out after {}ms",
                    Self::RECREATE_ONSCREEN_TIMEOUT.as_millis()
                ))),

            Err(e) => Err(EngineError::new(ErrorCode::Cancelled)
                .with_msg("recreate onscreen: recv failed")
                .with_detail(format!("{e:?}"))),
        }
    }

    pub(crate) fn shutdown(&mut self) {
        self.thread.shutdown();
    }
}
