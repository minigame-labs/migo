use shared::protocol::io_cmd::IOCmd;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tracing::info;

/// IO service — manages the channel to the IO handler thread.
///
/// The IO handler runs on a **dedicated OS thread** with its own
/// `current_thread` tokio runtime.  This is required because synchronous
/// file ops (`readFileSync`, `mkdirSync`, etc.) block the Host thread via
/// `crossbeam_channel::recv_timeout`.  If the IO handler shared the Host's
/// single-threaded runtime, it could never be polled while a sync op is
/// blocking — causing a guaranteed 10-second timeout (deadlock).
///
/// ## Lifecycle
///
/// 1. `IoService::new()` — creates the command channel (always succeeds).
/// 2. `IoService::spawn_handler()` — spawns the IO thread.
/// 3. `IoService::shutdown()` — sends `IOCmd::Shutdown` so the handler
///    exits its loop and calls `close_all()`.
pub(crate) struct IoService {
    tx: UnboundedSender<IOCmd>,
    /// IO command receiver, consumed by [`spawn_handler`].
    rx: Option<UnboundedReceiver<IOCmd>>,
    /// Join handle for the dedicated IO thread.
    handler_handle: Option<std::thread::JoinHandle<()>>,
}

impl IoService {
    /// Create the IO service channel.
    ///
    /// The handler is **not** started yet; call [`spawn_handler`] to launch
    /// the dedicated IO thread.  This separation allows `Host::new()` to
    /// create the service before the runtime enters `block_on`.
    pub(crate) fn new() -> Self {
        let (tx, rx) = unbounded_channel();
        Self {
            tx,
            rx: Some(rx),
            handler_handle: None,
        }
    }

    /// Spawn the IO command handler on a dedicated OS thread.
    ///
    /// Must be called **exactly once**.  The handler gets its own lightweight
    /// `current_thread` tokio runtime so it can process commands even when
    /// the Host thread is blocked by synchronous file operations.
    pub(crate) fn spawn_handler(&mut self) {
        let rx = self
            .rx
            .take()
            .expect("[BUG] IoService::spawn_handler called twice");
        let handle = std::thread::Builder::new()
            .name("Migo-IO".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .max_blocking_threads(4)
                    .build()
                    .expect("failed to create IO runtime");
                rt.block_on(io::run_io_handler(rx));
            })
            .expect("failed to spawn IO thread");
        self.handler_handle = Some(handle);
        info!("IO handler spawned on dedicated thread");
    }

    #[inline]
    pub(crate) fn sender(&self) -> UnboundedSender<IOCmd> {
        self.tx.clone()
    }

    /// Cooperative shutdown: send `Shutdown` so the handler exits its loop.
    ///
    /// The IO thread will process any remaining commands, call `close_all()`,
    /// and exit naturally.  We don't join here to avoid blocking the caller.
    /// File handles are cleaned up via their `Drop` impls.
    pub(crate) fn shutdown(&mut self) {
        let _ = self.tx.send(IOCmd::Shutdown);
        // Drop the join handle — the thread will exit on its own after
        // processing the Shutdown command.
        self.handler_handle.take();
    }
}
