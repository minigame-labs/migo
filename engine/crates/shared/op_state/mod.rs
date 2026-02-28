use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use crate::channel::ThreadWakeup;
use crate::protocol::{audio_cmd::AudioCmd, io_cmd::IOCmd, render_cmd::RenderCommand};
use crate::services::DeviceServices;
use crate::vfs::{GamePaths, VirtualFS};

/// Host-side operational state shared across runtime layers.
pub type RenderTx = crossbeam_channel::Sender<RenderCommand>;
pub type IoTx = UnboundedSender<IOCmd>;
pub type AudioTx = AudioSender;

/// Receiver for RAF (requestAnimationFrame) frame signals from the render thread.
/// Wrapped in Arc<Mutex> so the async op can lock and recv without borrowing OpState.
pub type RafRx = Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<f64>>>;

/// A wrapper around `UnboundedSender<AudioCmd>` that automatically notifies
/// the audio thread's [`ThreadWakeup`] on every send.
///
/// This ensures the audio thread wakes up immediately from any power-save
/// sleep state (LowPower / Sleep) when a new command arrives, keeping
/// audio-start latency consistently low (< 1ms for wakeup).
#[derive(Clone)]
pub struct AudioSender {
    tx: UnboundedSender<AudioCmd>,
    wakeup: ThreadWakeup,
}

impl AudioSender {
    /// Create a new `AudioSender` that wraps the given channel + wakeup.
    pub fn new(tx: UnboundedSender<AudioCmd>, wakeup: ThreadWakeup) -> Self {
        Self { tx, wakeup }
    }

    /// Send a command to the audio thread and immediately signal wakeup.
    #[inline]
    pub fn send(
        &self,
        value: AudioCmd,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<AudioCmd>> {
        let result = self.tx.send(value);
        // Always notify — even if the send fails (thread may still be draining).
        self.wakeup.notify();
        result
    }
}

impl fmt::Debug for AudioSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioSender").finish()
    }
}

#[derive(Clone)]
pub struct HostOpState {
    pub id: i32,
    /// App-level cache directory (Context.getCacheDir()).
    pub app_cache_dir: PathBuf,
    /// App-level files directory (Context.getFilesDir()).
    pub app_files_dir: PathBuf,
    /// Game code directory (set after EvaluateModule).
    pub code_dir: Option<String>,
    /// Game-specific paths (set after EvaluateModule).
    pub game_paths: Option<Arc<GamePaths>>,
    /// Virtual file system for path sandboxing (set after EvaluateModule).
    pub vfs: Option<Arc<VirtualFS>>,
    pub render_tx: RenderTx,
    pub io_tx: IoTx,
    pub audio_tx: AudioTx,
    /// Platform device services (clipboard, sensors, etc.)
    pub device_services: Option<Arc<dyn DeviceServices>>,
    /// RAF frame signal receiver (set by Host::new, consumed by op_await_next_frame).
    pub raf_rx: Option<RafRx>,
}

impl fmt::Debug for HostOpState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostOpState")
            .field("id", &self.id)
            .field("app_cache_dir", &self.app_cache_dir)
            .field("app_files_dir", &self.app_files_dir)
            .field("code_dir", &self.code_dir)
            .field("game_paths", &self.game_paths)
            .field("vfs", &self.vfs)
            .field("device_services", &self.device_services.as_ref().map(|_| "..."))
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct CanvasOpState {
    pub tx: RenderTx,
    pub has_onscreen: bool,
}

impl CanvasOpState {
    #[inline]
    pub fn new(tx: RenderTx, has_onscreen: bool) -> Self {
        Self { tx, has_onscreen }
    }
}
