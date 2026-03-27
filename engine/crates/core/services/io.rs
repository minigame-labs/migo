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

    /// Cooperative shutdown: send `Shutdown` and join with a timeout.
    ///
    /// The IO thread will process any remaining commands, call `close_all()`,
    /// and exit naturally.  We join with a 2-second timeout to ensure a clean
    /// shutdown without blocking the caller indefinitely.
    pub(crate) fn shutdown(&mut self) {
        let _ = self.tx.send(IOCmd::Shutdown);
        if let Some(handle) = self.handler_handle.take() {
            // Park on the IO thread for up to 2 seconds. This mirrors the
            // pattern used by the audio and render thread shutdown paths,
            // ensuring pending IO ops have time to complete gracefully.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                if handle.is_finished() {
                    let _ = handle.join();
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    tracing::warn!("IO thread did not exit within 2s, detaching");
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
}
