use shared::protocol::io_cmd::IOCmd;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{info, warn};

use crate::io_cmd_handler::IoCmdHandler;

/// Async IO command processing loop.
///
/// Runs as a `tokio::spawn` task on the **Host runtime**, eliminating the need
/// for a separate IO tokio runtime instance (P1-2: unified runtime).
///
/// ## Why this is safe for the JS event loop
///
/// All heavy work is offloaded to tokio's blocking thread pool:
/// - `tokio::fs::*` calls `spawn_blocking` internally
/// - Image decode / zip extract use explicit `spawn_blocking`
///
/// The handler itself only does lightweight dispatching: receive command →
/// kick off async op → await result → send response.  Between each `.await`
/// the executor polls other tasks (JS event loop, RAF, timers).
///
/// ## Backpressure
///
/// The channel is unbounded (same as before).  The handler batches up to
/// `MAX_BATCH` commands per wake to avoid one producer monopolising the loop.
pub async fn run_io_handler(mut rx: UnboundedReceiver<IOCmd>) {
    let mut handler = IoCmdHandler::new();

    // Batching prevents one producer from monopolizing the loop and starving others.
    const MAX_BATCH: usize = 256;

    'outer: loop {
        // Wait for at least one command.
        let first = match rx.recv().await {
            Some(cmd) => cmd,
            None => {
                warn!("IO handler: channel closed, exiting");
                break 'outer;
            }
        };

        if matches!(first, IOCmd::Shutdown) {
            info!("IO handler received Shutdown");
            break 'outer;
        }

        handler.handle_cmd(first).await;

        // Drain up to MAX_BATCH - 1 more commands opportunistically.
        for _ in 0..(MAX_BATCH - 1) {
            match rx.try_recv() {
                Ok(cmd) => {
                    if matches!(cmd, IOCmd::Shutdown) {
                        info!("IO handler received Shutdown");
                        break 'outer;
                    }
                    handler.handle_cmd(cmd).await;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    warn!("IO handler: channel disconnected, exiting");
                    break 'outer;
                }
            }
        }
    }

    handler.close_all();
    info!("IO handler stopped");
}
