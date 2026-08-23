use std::time::Duration;

use crossbeam_channel::{RecvTimeoutError, bounded};
use tracing::{info, warn};

use graphics::{RenderThread, SurfaceSystem};

use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    protocol::render_cmd::{CanvasCmd, RenderCmdResp, RenderCommand},
    render_event::RenderEventReceiver,
    surface::{PixelRatio, SurfaceGeneration, SurfaceLease},
};

use super::{SurfaceAttachmentSlot, SurfaceTransitionError};

pub(crate) struct RenderService {
    attachment: SurfaceAttachmentSlot,
    surface_system: SurfaceSystem,
    thread: RenderThread,
}

fn surface_for_restore(lease: Option<SurfaceLease>) -> EngineResult<SurfaceLease> {
    lease.ok_or_else(|| {
        EngineError::new(ErrorCode::InvalidOperation)
            .with_msg("restore surface: no live surface available")
    })
}

fn transition_error(context: &'static str, error: SurfaceTransitionError) -> EngineError {
    EngineError::new(ErrorCode::InvalidOperation)
        .with_msg(context)
        .with_detail(error.to_string())
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
        frame_demand_rx: Option<crossbeam_channel::Receiver<()>>,
        host_id: i32,
        // `None` when the session is created before its window Surface
        // exists. Everything the render thread does that a Surface is not
        // needed for -- EGL display and config, the pbuffer resource context,
        // the GLES dispatch table, capability detection, Skia -- then runs
        // while the host application is still laying out its window, instead
        // of after. Measured on a Mate 30 Pro that is ~50 ms taken off the
        // path to first frame, in a window that was provably idle: an
        // Activity that rotates to a landscape game sits 150 ms between
        // `onCreate` and `surfaceCreated` doing nothing the engine could not
        // have been doing.
        initial_surface: Option<SurfaceLease>,
        graphics_platform: graphics::egl_platform::GraphicsPlatform,
        pixel_ratio: f32,
        target_fps: i32,
        app_cache_dir: Option<std::path::PathBuf>,
        gpu_caps: std::sync::Arc<shared::device::gpu_caps::GpuCaps>,
        context_lost: std::sync::Arc<shared::op_state::ContextLostState>,
        wake: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
        raf_demand: shared::raf_signal::RafDemandRef,
        request_vsync: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
        surface_control: std::sync::Arc<shared::surface::SurfaceControl>,
        report_surface_loss: std::sync::Arc<
            dyn Fn(shared::surface::PublicSurfaceGeneration, shared::surface::SurfaceLossReason)
                + Send
                + Sync,
        >,
    ) -> EngineResult<Self> {
        let surface_size = initial_surface.as_ref().map(|lease| lease.size());

        let thread = RenderThread::spawn(
            raf_tx,
            vsync_rx,
            frame_demand_rx,
            host_id,
            initial_surface.clone(),
            graphics_platform,
            pixel_ratio,
            app_cache_dir,
            gpu_caps,
            // Resolved here rather than inside the render thread: a GPU startup
            // timeout detaches that thread without joining it, and the host's
            // startup guard unregisters this cache, so a get-or-create reached
            // after that point would leak a registry entry per failed startup.
            shared::text_texture_cache::text_cache_for_host(host_id),
            context_lost,
            wake,
            raf_demand,
            request_vsync,
            surface_control,
            report_surface_loss,
        )?;
        // Apply the host's configured target FPS to the render thread immediately
        // so the first vsync tick already runs at the right cadence.
        let _ = thread
            .sender()
            .send(RenderCommand::FrameRate(target_fps.clamp(1, 120) as u32));
        let mut surface_system = SurfaceSystem::new();
        if let Some(surface_size) = surface_size {
            surface_system.on_surface_available(surface_size);
        }
        Ok(Self {
            attachment: match initial_surface {
                Some(lease) => SurfaceAttachmentSlot::from_initial(lease),
                // Not "detached after having been attached": never attached.
                // `update_surface` reaches `install_surface_lease` with
                // `binding.is_live()` false either way, so the first Surface a
                // warm-started session receives takes the same install path an
                // initial one would have.
                None => SurfaceAttachmentSlot::empty(),
            },
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
        self.attachment.has_live_surface()
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
    pub(crate) fn update_surface(
        &mut self,
        lease: SurfaceLease,
        pixel_ratio: Option<PixelRatio>,
    ) -> EngineResult<()> {
        self.attachment
            .prepare(&lease)
            .map_err(|error| transition_error("recreate onscreen: rejected Surface", error))?;
        let surface_size = lease.size();

        let (tx, rx) = bounded::<Result<(), EngineError>>(1);
        let cmd = RenderCommand::Canvas(CanvasCmd::RecreateOnscreen {
            lease: lease.clone(),
            pixel_ratio,
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
                // Retirement is synchronous and independent of the command
                // response. Never publish a ready Host attachment if destroy
                // raced with EGL recreation.
                if !lease.is_live() {
                    self.surface_system.on_surface_destroyed();
                    return Err(EngineError::new(ErrorCode::Cancelled)
                        .with_msg("recreate onscreen: Surface retired before commit"));
                }
                self.attachment.commit(lease).map_err(|error| {
                    self.surface_system.on_surface_destroyed();
                    transition_error("recreate onscreen: commit rejected Surface", error)
                })?;
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
        // Bounded-blocking: dropping Pause/Resume on a full render queue
        // desynchronizes lifecycle state and can leave the app frozen.
        let _ = self.sender().send_blocking_bounded(RenderCommand::Pause);
    }

    /// Record surface loss and clear any stale surface handle.
    pub(crate) fn on_surface_destroyed(&mut self, generation: SurfaceGeneration) {
        // Only the exact current generation may cross this bridge. A delayed
        // destroy for an older attachment cannot invalidate a newer Surface.
        if !self.attachment.detach(generation) {
            return;
        }
        self.surface_system.on_surface_destroyed();
        // Deliberately override SurfaceDestroyed's drop-on-full lifecycle
        // policy so render-side state converges promptly. Presentation safety
        // does not depend on delivery: the retired generation token is the
        // queue-independent barrier checked at every present boundary.
        let _ = self
            .sender()
            .send_blocking_bounded(RenderCommand::SurfaceDestroyed { generation });
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
        self.update_surface(surface_for_restore(self.attachment.live_lease())?, None)
    }

    pub(crate) fn shutdown(&mut self) {
        self.thread.shutdown();
    }

    pub(crate) fn shutdown_detached(&mut self) {
        self.thread.shutdown_detached();
    }
}

impl Drop for RenderService {
    fn drop(&mut self) {
        // `Host::drop` performs the normal joined shutdown first. This fallback
        // only has work on partial construction, `?` returns, or unwinding.
        self.shutdown_detached();
    }
}
