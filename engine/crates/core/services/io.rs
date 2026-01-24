use io::IOThread;

use shared::error::EngineResult;
use shared::protocol::io_cmd::IOCmd;

pub(crate) struct IoService {
    thread: IOThread,
}

impl IoService {
    pub(crate) fn new() -> EngineResult<Self> {
        let thread = IOThread::spawn()?;
        Ok(Self { thread })
    }

    #[inline]
    pub(crate) fn sender(&self) -> tokio::sync::mpsc::UnboundedSender<IOCmd> {
        self.thread.sender()
    }

    pub(crate) fn shutdown(&mut self) {
        self.thread.shutdown();
    }
}
