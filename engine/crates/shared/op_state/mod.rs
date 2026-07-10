use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tokio::sync::mpsc::UnboundedSender;

use crate::channel::ThreadWakeup;
use crate::protocol::audio_cmd::AudioCmd;
use crate::services::DeviceServices;
use crate::vfs::{GamePaths, MountTable, VirtualFS};

/// Host-side operational state shared across runtime layers.
pub type RenderTx = crate::render_command_sender::CommandSender;
pub type AudioTx = AudioSender;
pub type HostTx = crate::host_channel::HostCommandSender;

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
    /// F-2: optional shared-measurer handle, cloned at startup
    /// from `RenderThread::text_measurer()`.  Forwarded into
    /// `CanvasOpState::with_text_measurer` so JS-side
    /// `op_measure_text_flat` can measure without a cross-thread
    /// round-trip.  `None` in test harnesses that wire
    /// `HostOpState` manually.
    pub text_measurer: Option<crate::text_measurer::SharedTextMeasurer>,
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
    /// True after the first WebGL context is constructed in this runtime.
    ///
    /// Image decode policy uses this to choose the industrial fast path:
    /// Canvas2D-only games may keep AHB / compressed GPU-native images for
    /// zero-copy `drawImage`, while WebGL games need `Image` objects to retain
    /// CPU RGBA backing because `texImage2D(image)` is synchronous and uploads
    /// into the caller's currently-bound texture.  The flag is monotonic per
    /// runtime, matching browsers where WebGL-capable pages keep decoded image
    /// pixels available for texture uploads.
    pub webgl_context_created: Arc<AtomicBool>,
    /// Render-context lost flag, shared with the render-event consumer.
    ///
    /// Set `true` when the render thread reports `RenderEvent::ContextLost`
    /// and back to `false` on a successful `ContextRecovered`. Read by
    /// `op_gl_is_context_lost` so JS `gl.isContextLost()` reflects reality
    /// instead of a hard-coded `false` — games that guard draw calls on
    /// `isContextLost()` then stop issuing GL into a dead context.
    ///
    /// Written exclusively by the render thread (see [`ContextLostState`]); the
    /// host reads it to reconcile JS lifecycle events and JS reads `.lost` via
    /// `op_gl_is_context_lost`.
    pub context_lost: Arc<ContextLostState>,
    /// Whether code signing enforcement is enabled for this runtime.
    pub code_signing_enabled: bool,
    /// Per-session GPU compressed texture format support.
    /// Shared with the render thread via `Arc`; render thread calls
    /// `set()` after GL context init.  JS ops take a `snapshot()` and
    /// pass it to image decode functions.
    pub gpu_caps: Arc<crate::device::gpu_caps::GpuCaps>,
}

/// Shared GL context-loss state. The render thread is the **sole writer** and
/// updates it edge-triggered: on a real loss `lost` flips false→true and the
/// epoch bumps; on a probe-verified recovery it flips true→false and the epoch
/// bumps.
///
/// The `(lost, epoch)` pair is packed into a **single** `AtomicU64` (bit 0 =
/// `lost`, bits 1.. = `epoch`) so a reader always gets a *consistent snapshot*.
/// A previous two-atomic design allowed a torn read in the window between the
/// render thread writing `lost` and bumping `epoch`, which made the host emit a
/// spurious `restored, lost, restored` sequence. A single atomic eliminates
/// that: the render thread (sole writer) does a plain load+store RMW — no CAS
/// needed — and the host does one `Acquire` load.
///
/// The epoch exists because the render-event channel is lossy (bounded, drops
/// on full): if a `ContextLost`/`ContextRecovered` notification is dropped, the
/// host still detects that a transition happened by observing the epoch jump
/// and synthesizes the missing `webglcontextlost`/`webglcontextrestored` pair.
/// A bare `lost` bool alone can only converge to the final level and would
/// silently swallow a lost→recovered edge pair.
///
/// JS `op_gl_is_context_lost` reads the `lost` bit via [`Self::is_lost`].
#[derive(Debug, Default)]
pub struct ContextLostState {
    /// Packed `(epoch << 1) | (lost as u64)`.
    packed: AtomicU64,
}

impl ContextLostState {
    /// Read a consistent `(lost, epoch)` snapshot (host reconcile path).
    #[inline]
    pub fn snapshot(&self) -> (bool, u64) {
        let v = self.packed.load(Ordering::Acquire);
        (v & 1 == 1, v >> 1)
    }

    /// Read just the current lost level (JS `op_gl_is_context_lost`). A single
    /// bool query tolerates `Relaxed`.
    #[inline]
    pub fn is_lost(&self) -> bool {
        self.packed.load(Ordering::Relaxed) & 1 == 1
    }

    /// Render thread only: transition to lost. Returns `true` iff this was the
    /// false→true edge (so the caller emits the event + nudges exactly once).
    #[inline]
    pub fn set_lost(&self) -> bool {
        let cur = self.packed.load(Ordering::Acquire);
        if cur & 1 == 1 {
            return false; // already lost — no edge
        }
        let epoch = cur >> 1;
        self.packed.store(((epoch + 1) << 1) | 1, Ordering::Release);
        true
    }

    /// Render thread only: transition to recovered. Returns `true` iff this was
    /// the true→false edge.
    #[inline]
    pub fn set_recovered(&self) -> bool {
        let cur = self.packed.load(Ordering::Acquire);
        if cur & 1 == 0 {
            return false; // already recovered — no edge
        }
        let epoch = cur >> 1;
        self.packed.store((epoch + 1) << 1, Ordering::Release);
        true
    }
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

#[derive(Clone)]
pub struct CanvasOpState {
    pub tx: RenderTx,
    /// F-2: optional shared-measurer handle.  When present, JS
    /// ops (`op_measure_text_flat`, `op_get_text_line_height`)
    /// call the trait directly and skip the `RenderCommand::
    /// Canvas2D { MeasureText }` round-trip entirely.  The
    /// graphics crate registers the handle during render-thread
    /// bring-up via `RenderThread::text_measurer()`; host
    /// runtimes that use the engine headlessly (tests, tooling)
    /// can leave this `None` and the ops fall back to the
    /// cross-thread sync-op path.
    pub text_measurer: Option<crate::text_measurer::SharedTextMeasurer>,
}

impl std::fmt::Debug for CanvasOpState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanvasOpState")
            .field("tx", &self.tx)
            .field(
                "text_measurer",
                &self
                    .text_measurer
                    .as_ref()
                    .map(|_| "Some(<dyn TextMeasurer>)"),
            )
            .finish()
    }
}

impl CanvasOpState {
    #[inline]
    pub fn new(tx: RenderTx) -> Self {
        Self {
            tx,
            text_measurer: None,
        }
    }

    /// Attach a `TextMeasurer` after construction.  Called by the
    /// host runtime wiring layer once the render thread has
    /// published its shared handle.
    #[inline]
    pub fn with_text_measurer(
        mut self,
        measurer: crate::text_measurer::SharedTextMeasurer,
    ) -> Self {
        self.text_measurer = Some(measurer);
        self
    }
}
