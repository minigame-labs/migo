use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use crate::channel::ThreadWakeup;
use crate::protocol::{audio_cmd::AudioCmd, host_cmd::HostCommand, io_cmd::IOCmd};
use crate::services::DeviceServices;
use crate::vfs::{GamePaths, MountTable, VirtualFS};

/// Host-side operational state shared across runtime layers.
pub type RenderTx = crate::render_command_sender::CommandSender;
pub type IoTx = UnboundedSender<IOCmd>;
pub type AudioTx = AudioSender;
pub type HostTx = tokio::sync::mpsc::Sender<HostCommand>;

/// Receiver for RAF (requestAnimationFrame) frame signals from the render thread.
///
/// On Android: backed by eventfd (low-latency epoll wake).
/// Other platforms: backed by tokio mpsc channel.
///
/// The concrete type is `graphics::raf_signal::RafReceiver` which handles
/// both variants internally.  Wrapped in Arc for restart survival.
pub type RafRx = Arc<crate::raf_signal::RafReceiver>;

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
    /// Mount table for `/code` path resolution (set after EvaluateModule).
    pub mount_table: Option<Arc<MountTable>>,
    pub render_tx: RenderTx,
    pub io_tx: IoTx,
    pub audio_tx: AudioTx,
    pub host_tx: HostTx,
    /// Platform device services (clipboard, sensors, etc.)
    pub device_services: Option<Arc<dyn DeviceServices>>,
    /// RAF frame signal receiver (set by Host::new, consumed by op_await_next_frame).
    pub raf_rx: Option<RafRx>,
    /// Subpackage definitions: (name, root) pairs from RuntimeConfig.
    pub sub_packages: Vec<(String, String)>,
    /// Workers directory path from RuntimeConfig.
    pub workers_path: Option<String>,
    /// Network security policy (domain whitelist, HTTPS enforcement).
    pub network_policy: NetworkPolicy,
    /// When `true`, the app is in the background (OnHide received).
    /// Async polling loops (WebSocket, TCP, UDP) check this flag and
    /// throttle their iteration rate to reduce CPU/battery usage.
    pub backgrounded: Arc<AtomicBool>,
    /// Whether code signing enforcement is enabled for this runtime.
    pub code_signing_enabled: bool,
    /// Per-session GPU compressed texture format support.
    /// Shared with the render thread via `Arc`; render thread calls
    /// `set()` after GL context init.  JS ops take a `snapshot()` and
    /// pass it through `IOCmd` so the IO thread never reads globals.
    pub gpu_caps: Arc<crate::device::gpu_caps::GpuCaps>,
}

/// Network-level security policy, populated from InitOptions.extras.
#[derive(Debug, Clone, Default)]
pub struct NetworkPolicy {
    /// When non-empty, only these domains (and their subdomains) may be accessed.
    /// An empty Vec means "allow all domains" (no whitelist).
    pub domain_whitelist: Vec<String>,
    /// When true, only HTTPS URLs are allowed for fetch/upload (HTTP rejected).
    pub enforce_https: bool,
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
            .field("mount_table", &self.mount_table.as_ref().map(|_| "..."))
            .field(
                "device_services",
                &self.device_services.as_ref().map(|_| "..."),
            )
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct CanvasOpState {
    pub tx: RenderTx,
}

impl CanvasOpState {
    #[inline]
    pub fn new(tx: RenderTx) -> Self {
        Self { tx }
    }
}
