use audio::AudioThread;

use shared::error::EngineResult;
use shared::protocol::audio_cmd::AudioCmd;
use shared::protocol::host_cmd::HostCommand;
use tokio::sync::mpsc::UnboundedSender;

pub(crate) struct AudioService {
    thread: AudioThread,
}

impl AudioService {
    pub(crate) fn new(host_tx: tokio::sync::mpsc::Sender<HostCommand>) -> EngineResult<Self> {
        let thread = AudioThread::spawn(host_tx)?;
        Ok(Self { thread })
    }

    #[inline]
    pub(crate) fn sender(&self) -> UnboundedSender<AudioCmd> {
        self.thread.sender()
    }

    /// Pause all audio processing (app going to background).
    pub(crate) fn pause(&self) {
        let _ = self.sender().send(AudioCmd::PauseAll);
    }

    /// Resume all audio processing (app returning to foreground).
    pub(crate) fn resume(&self) {
        let _ = self.sender().send(AudioCmd::ResumeAll);
    }

    pub(crate) fn shutdown(&mut self) {
        self.thread.shutdown();
    }
}
