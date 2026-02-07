use std::thread;

use shared::error::{EngineError, EngineResult, ErrorCode};
use shared::protocol::io_cmd::IOCmd;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tracing::{error, info, warn};

use crate::io_cmd_handler::IoCmdHandler;

/// Result of runtime initialization, sent back to spawner
enum RuntimeInitResult {
    Ok(thread::ThreadId),
    Err(std::io::Error),
}

pub struct IOThread {
    tx: UnboundedSender<IOCmd>,
    handle: Option<thread::JoinHandle<()>>,
    io_thread_id: thread::ThreadId,
}

impl IOThread {
    pub fn spawn() -> EngineResult<Self> {
        let (tx, mut rx) = unbounded_channel::<IOCmd>();

        // Handshake channel to obtain the spawned thread's ThreadId and init result.
        let (init_tx, init_rx) = std::sync::mpsc::sync_channel::<RuntimeInitResult>(1);

        let handle = thread::Builder::new()
            .name("Migo-IOThread".into())
            .spawn(move || {
                // Build runtime first, report any error back to spawner
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .enable_time()
                    .max_blocking_threads(8)
                    .build()
                {
                    Ok(rt) => {
                        // Send success with thread id
                        let _ = init_tx.send(RuntimeInitResult::Ok(thread::current().id()));
                        rt
                    }
                    Err(e) => {
                        // Send error back to spawner
                        error!("Failed to build IO tokio runtime: {:?}", e);
                        let _ = init_tx.send(RuntimeInitResult::Err(e));
                        return;
                    }
                };

                info!("IOThread started");

                rt.block_on(async move {
                    let mut handler = IoCmdHandler::new();

                    // Batching prevents one producer from monopolizing the loop and starving others.
                    const MAX_BATCH: usize = 256;

                    'outer: loop {
                        // Wait for at least one command.
                        let first = match rx.recv().await {
                            Some(cmd) => cmd,
                            None => {
                                warn!("IOThread channel closed, exiting");
                                break 'outer;
                            }
                        };

                        if matches!(first, IOCmd::Shutdown) {
                            info!("IOThread received Shutdown");
                            break 'outer;
                        }

                        handler.handle_cmd(first).await;

                        // Drain up to MAX_BATCH - 1 more commands opportunistically.
                        for _ in 0..(MAX_BATCH - 1) {
                            match rx.try_recv() {
                                Ok(cmd) => {
                                    if matches!(cmd, IOCmd::Shutdown) {
                                        info!("IOThread received Shutdown");
                                        break 'outer;
                                    }
                                    handler.handle_cmd(cmd).await;
                                }
                                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                                    warn!("IOThread channel disconnected, exiting");
                                    break 'outer;
                                }
                            }
                        }
                    }

                    handler.close_all();
                    info!("IOThread stopped");
                });
            })
            .map_err(|e| EngineError::from_detail(ErrorCode::IoError, format!("Failed to spawn IO thread: {}", e)))?;

        // Blocking wait for init result from the spawned thread.
        match init_rx.recv() {
            Ok(RuntimeInitResult::Ok(io_thread_id)) => Ok(Self {
                tx,
                handle: Some(handle),
                io_thread_id,
            }),
            Ok(RuntimeInitResult::Err(e)) => Err(EngineError::from_detail(
                ErrorCode::IoError,
                format!("Failed to build tokio runtime: {}", e),
            )),
            Err(_) => Err(EngineError::from_detail(
                ErrorCode::IoError,
                "IO thread terminated before sending init result",
            )),
        }
    }

    #[inline]
    pub fn sender(&self) -> UnboundedSender<IOCmd> {
        self.tx.clone()
    }

    pub fn shutdown(&mut self) {
        let _ = self.tx.send(IOCmd::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for IOThread {
    fn drop(&mut self) {
        // Best-effort shutdown.
        let _ = self.tx.send(IOCmd::Shutdown);

        // Never join from inside the IO thread itself (would deadlock).
        if thread::current().id() == self.io_thread_id {
            self.handle.take();
            return;
        }

        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
