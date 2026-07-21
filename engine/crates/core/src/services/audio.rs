#[cfg(feature = "api-media")]
use audio::AudioThread;

#[cfg(feature = "api-media")]
use shared::channel::ThreadWakeup;
#[cfg(feature = "api-media")]
use shared::error::{EngineError, EngineResult, ErrorCode};
#[cfg(feature = "api-media")]
use shared::op_state::{AudioHostStartSignal, AudioSender, HostTx};
#[cfg(feature = "api-media")]
use shared::protocol::audio_cmd::AudioCmd;
#[cfg(feature = "api-media")]
use std::sync::Arc;
#[cfg(feature = "api-media")]
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
#[cfg(feature = "api-media")]
use tracing::info;

/// Lazy audio service — the actual `AudioThread` is not spawned until the
/// first real audio command arrives.
///
/// Creating the `AudioThread` during `Host::new()` was one of the most
/// expensive steps (~50–100 ms on mid-range devices) because it:
///   - Initialises the `cpal` audio output (enumerates devices, opens stream)
///   - Allocates ring buffers, resampler state, etc.
///
/// Most mini-games do not use audio until well after first frame (e.g.,
/// background music starts after splash screen).  By deferring thread
/// creation until the first `AudioCmd`, we remove ~80 ms from cold-start.
///
/// ## Implementation
///
/// 1. `AudioService::new()` creates the channel, wakeup, and a policy-capturing
///    HTTP factory closure. It does not build a reqwest client/pool or thread.
/// 2. `sender()` returns an [`AudioSender`] backed by the channel.
/// 3. A successful pre-start command notifies the Host event loop, which calls
///    `check_and_start()`. It drains queued commands and, upon the first *real*
///    audio command (i.e. not PauseAll/ResumeAll/Shutdown), spawns the
///    real `AudioThread` via [`AudioThread::spawn_with_channel`] which
///    re-uses the **same channel** — no forwarding task needed.
#[cfg(feature = "api-media")]
pub(crate) struct AudioService {
    /// Sender end of the channel.  Ops write here immediately.
    tx: UnboundedSender<AudioCmd>,
    /// Wakeup handle shared with [`AudioSender`] instances.
    wakeup: ThreadWakeup,
    /// Receiver end — held until the thread is started, then handed off.
    rx: Option<UnboundedReceiver<AudioCmd>>,
    /// Commands dequeued before the thread starts (replayed on start).
    pending: Vec<AudioCmd>,
    /// Handle to the spawned `AudioThread` (Some once started).
    thread: Option<AudioThread>,
    /// Host command sender — needed to construct `AudioThread`.
    host_tx: HostTx,
    /// Tracks whether the app is currently paused (OnHide received).
    /// If the thread is lazily started while paused, we immediately
    /// send `PauseAll` to avoid audio playing in the background.
    is_paused: bool,
    /// Coalescing lazy-audio host-start signal. Handed to every host
    /// `AudioSender` so a pre-start command nudges the host loop to call
    /// `check_and_start()`, and disabled (`mark_started`) once the thread runs.
    /// Replaces the lazy-audio poll that the deleted 3-second heartbeat did.
    host_start: Arc<AudioHostStartSignal>,
    /// Per-host factory. The audio thread invokes it only on the first remote
    /// cache miss, then keeps the resulting reqwest pool for its lifetime.
    http_client_factory: audio::streaming::StreamingHttpClientFactory,
}

#[cfg(feature = "api-media")]
impl AudioService {
    /// Create a lazy audio service. **No thread or HTTP client is created.**
    pub(crate) fn new(host_tx: HostTx, network_policy: shared::op_state::NetworkPolicy) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let wakeup = ThreadWakeup::new();
        let http_client_factory: audio::streaming::StreamingHttpClientFactory =
            Arc::new(move || {
                runtime_v8::create_audio_http_client(&network_policy).map_err(|error| {
                    EngineError::from_detail(
                        ErrorCode::IoError,
                        format!("failed to build audio HTTP client: {error}"),
                    )
                })
            });
        Self {
            tx,
            wakeup,
            rx: Some(rx),
            pending: Vec::new(),
            thread: None,
            host_tx,
            is_paused: false,
            host_start: AudioHostStartSignal::new(),
            http_client_factory,
        }
    }

    /// Return an [`AudioSender`] that auto-wakes the audio thread on each
    /// send.  Safe to call before the thread is started — commands queue up
    /// and are replayed once the thread spawns.  Carries the lazy-audio
    /// host-start signal so a pre-start send nudges the host loop.
    #[inline]
    pub(crate) fn sender(&self) -> AudioSender {
        AudioSender::with_host_start_signal(
            self.tx.clone(),
            self.wakeup.clone(),
            self.host_start.clone(),
        )
    }

    /// The lazy-audio host-start signal, for the host event loop to select on.
    #[inline]
    pub(crate) fn start_signal(&self) -> Arc<AudioHostStartSignal> {
        self.host_start.clone()
    }

    /// Called from the Host event loop when the coalescing start signal fires.
    ///
    /// Drains queued commands from the channel and starts the audio thread
    /// on the first "real" command (anything except PauseAll/ResumeAll/
    /// Shutdown).  Cost when thread is already running: one branch.
    pub(crate) fn check_and_start(&mut self) -> EngineResult<()> {
        if self.thread.is_some() {
            return Ok(());
        }

        if let Some(ref mut rx) = self.rx {
            while let Ok(cmd) = rx.try_recv() {
                let is_lifecycle = matches!(
                    cmd,
                    AudioCmd::PauseAll | AudioCmd::ResumeAll | AudioCmd::Shutdown
                );
                self.pending.push(cmd);
                if !is_lifecycle {
                    // A real audio command arrived — start the thread now.
                    return self.start_thread();
                }
            }
        }
        Ok(())
    }

    /// Force-start the thread even if no real command has arrived.
    ///
    /// Used by `on_restart()` which needs a live thread for ResumeAll.
    #[allow(dead_code)]
    pub(crate) fn ensure_started(&mut self) -> EngineResult<()> {
        if self.thread.is_some() {
            return Ok(());
        }
        self.start_thread()
    }

    fn start_thread(&mut self) -> EngineResult<()> {
        info!(
            "AudioService: lazily starting audio thread ({} buffered cmds)",
            self.pending.len()
        );

        // Re-inject buffered commands into the channel so the thread sees
        // them when it starts consuming from `rx`.  This is safe because
        // no other consumer exists for `rx` at this point.
        for cmd in self.pending.drain(..) {
            let _ = self.tx.send(cmd);
        }

        // If the app is currently paused (OnHide), inject PauseAll so the
        // new thread doesn't play audio while backgrounded.
        if self.is_paused {
            let _ = self.tx.send(AudioCmd::PauseAll);
        }

        // Hand the receiver + wakeup directly to the thread — the thread
        // reads from the same channel that ops write to.  No forwarding
        // task needed.
        let rx = self
            .rx
            .take()
            .expect("[BUG] AudioService::start_thread called twice");
        let thread = AudioThread::spawn_with_channel(
            self.tx.clone(),
            rx,
            self.wakeup.clone(),
            self.host_tx.clone(),
            self.http_client_factory.clone(),
        )?;

        self.thread = Some(thread);
        // The real audio thread is installed: disable the host-start signal so
        // later sends stop nudging the host loop (release ordering, after the
        // thread is stored).
        self.host_start.mark_started();
        Ok(())
    }

    /// Pause all audio processing (app going to background).
    pub(crate) fn pause(&mut self) {
        self.is_paused = true;
        if self.thread.is_some() {
            let _ = self.tx.send(AudioCmd::PauseAll);
            self.wakeup.notify();
        }
        // If thread not started, no-op — nothing to pause.
        // `is_paused` is tracked so that if the thread starts later,
        // it immediately receives PauseAll.
    }

    /// Resume all audio processing (app returning to foreground).
    pub(crate) fn resume(&mut self) {
        self.is_paused = false;
        if self.thread.is_some() {
            let _ = self.tx.send(AudioCmd::ResumeAll);
            self.wakeup.notify();
        }
    }

    pub(crate) fn shutdown(&mut self) {
        if let Some(ref mut thread) = self.thread {
            thread.shutdown();
        }
    }
}

#[cfg(not(feature = "api-media"))]
pub(crate) struct AudioService {
    tx: tokio::sync::mpsc::UnboundedSender<shared::protocol::audio_cmd::AudioCmd>,
    _rx: tokio::sync::mpsc::UnboundedReceiver<shared::protocol::audio_cmd::AudioCmd>,
    wakeup: shared::channel::ThreadWakeup,
    start_signal: std::sync::Arc<shared::op_state::AudioHostStartSignal>,
}

#[cfg(not(feature = "api-media"))]
impl AudioService {
    pub(crate) fn new(
        _host_tx: shared::op_state::HostTx,
        _network_policy: shared::op_state::NetworkPolicy,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let start_signal = shared::op_state::AudioHostStartSignal::new();
        start_signal.mark_started();
        Self {
            tx,
            _rx: rx,
            wakeup: shared::channel::ThreadWakeup::new(),
            start_signal,
        }
    }

    #[inline]
    pub(crate) fn sender(&self) -> shared::op_state::AudioSender {
        shared::op_state::AudioSender::new(self.tx.clone(), self.wakeup.clone())
    }

    #[inline]
    pub(crate) fn start_signal(&self) -> std::sync::Arc<shared::op_state::AudioHostStartSignal> {
        self.start_signal.clone()
    }

    #[inline]
    pub(crate) fn check_and_start(&mut self) -> shared::error::EngineResult<()> {
        Ok(())
    }

    #[inline]
    pub(crate) fn pause(&mut self) {}

    #[inline]
    pub(crate) fn resume(&mut self) {}

    #[inline]
    pub(crate) fn shutdown(&mut self) {}
}
