use shared::protocol::io_cmd::IOCmd;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tracing::info;

/// IO service — manages the channel to the IO handler task.
///
/// The IO handler no longer runs on a separate thread with its own tokio
/// runtime.  Instead, it is spawned as a `tokio::spawn` task on the Host
/// runtime.  This eliminates one epoll fd, one timer wheel, and one set
/// of blocking thread pool resources.
///
/// ## Lifecycle
///
/// 1. `IoService::new()` — creates the command channel (always succeeds).
/// 2. `IoService::spawn_handler()` — spawns the handler task on the current
///    tokio runtime.  Must be called from within `runtime.block_on()`.
/// 3. `IoService::shutdown()` — sends `IOCmd::Shutdown` so the handler
///    exits its loop and calls `close_all()`.
pub(crate) struct IoService {
    tx: UnboundedSender<IOCmd>,
    /// IO command receiver, consumed by [`spawn_handler`].
    rx: Option<UnboundedReceiver<IOCmd>>,
    /// Join handle for the IO handler task.
    handler_handle: Option<tokio::task::JoinHandle<()>>,
}

impl IoService {
    /// Create the IO service channel.
    ///
    /// The handler is **not** started yet; call [`spawn_handler`] from within
    /// a tokio runtime context.  This separation allows `Host::new()` to
    /// create the service before the runtime enters `block_on`.
    pub(crate) fn new() -> Self {
        let (tx, rx) = unbounded_channel();
        Self {
            tx,
            rx: Some(rx),
            handler_handle: None,
        }
    }

    /// Spawn the IO command handler as a tokio task on the current runtime.
    ///
    /// Must be called **exactly once**, from within `runtime.block_on()`.
    /// After this call the IO handler runs as a cooperative task on the
    /// Host thread's tokio executor, sharing the same epoll/timer resources.
    pub(crate) fn spawn_handler(&mut self) {
        let rx = self
            .rx
            .take()
            .expect("[BUG] IoService::spawn_handler called twice");
        let handle = tokio::spawn(io::run_io_handler(rx));
        self.handler_handle = Some(handle);
        info!("IO handler spawned as tokio task on Host runtime");
    }

    #[inline]
    pub(crate) fn sender(&self) -> UnboundedSender<IOCmd> {
        self.tx.clone()
    }

    /// Best-effort shutdown: send `Shutdown` and abort the handler task.
    ///
    /// The IO handler runs as a tokio task on the Host's **current-thread**
    /// runtime.  We cannot synchronously wait for it here (that would
    /// block the sole executor thread, preventing the task from being
    /// polled).  Instead we send `Shutdown` (cooperative exit) and then
    /// abort the task (forced cancellation).  File handles are cleaned
    /// up via their `Drop` impls.
    pub(crate) fn shutdown(&mut self) {
        let _ = self.tx.send(IOCmd::Shutdown);
        if let Some(handle) = self.handler_handle.take() {
            handle.abort();
        }
    }
}
