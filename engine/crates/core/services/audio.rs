use audio::AudioThread;

use shared::channel::ThreadWakeup;
use shared::error::EngineResult;
use shared::op_state::AudioSender;
use shared::protocol::audio_cmd::AudioCmd;
use shared::protocol::host_cmd::HostCommand;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tracing::{error, info};

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
/// 1. `AudioService::new()` only creates the channel + wakeup.
/// 2. `sender()` returns an [`AudioSender`] backed by the channel.
/// 3. `check_and_start()` is called from the Host event loop on every
///    iteration.  It drains queued commands and, upon the first *real*
///    audio command (i.e. not PauseAll/ResumeAll/Shutdown), spawns the
///    real `AudioThread` via [`AudioThread::spawn_with_channel`] which
///    re-uses the **same channel** — no forwarding task needed.
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
    host_tx: tokio::sync::mpsc::Sender<HostCommand>,
    /// Tracks whether the app is currently paused (OnHide received).
    /// If the thread is lazily started while paused, we immediately
    /// send `PauseAll` to avoid audio playing in the background.
    is_paused: bool,
}

impl AudioService {
    /// Create a lazy audio service.  **No thread is spawned.**
    pub(crate) fn new(host_tx: tokio::sync::mpsc::Sender<HostCommand>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let wakeup = ThreadWakeup::new();
        Self {
            tx,
            wakeup,
            rx: Some(rx),
            pending: Vec::new(),
            thread: None,
            host_tx,
            is_paused: false,
        }
    }

    /// Return an [`AudioSender`] that auto-wakes the audio thread on each
    /// send.  Safe to call before the thread is started — commands queue up
    /// and are replayed once the thread spawns.
    #[inline]
    pub(crate) fn sender(&self) -> AudioSender {
        AudioSender::new(self.tx.clone(), self.wakeup.clone())
    }

    /// Called from the Host event loop on **every iteration**.
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
        )?;

        self.thread = Some(thread);
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
