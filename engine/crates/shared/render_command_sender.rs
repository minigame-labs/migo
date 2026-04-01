//! Render command sender — wraps a crossbeam bounded channel.
//!
//! Lives in `shared` so both `graphics` (render_thread) and `core`/`js-runtime`
//! (producers) can reference the type without circular dependencies.

use crate::protocol::render_cmd::RenderCommand;

/// Default render command queue capacity.
/// 512 provides ~8ms of buffering at 60fps with typical command rates.
const CHANNEL_CAPACITY: usize = 512;

/// Producer-side handle for sending render commands.
#[derive(Clone)]
pub struct CommandSender {
    inner: crossbeam_channel::Sender<RenderCommand>,
}

impl std::fmt::Debug for CommandSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandSender")
            .field("len", &self.inner.len())
            .finish()
    }
}

impl CommandSender {
    /// Create a new sender/receiver pair.
    ///
    /// Returns `(sender, cmd_rx)`:
    /// - `sender`: clone and distribute to producers
    /// - `cmd_rx`: the render thread receives from this in its `select!` loop
    pub fn new() -> (Self, crossbeam_channel::Receiver<RenderCommand>) {
        let (tx, rx) = crossbeam_channel::bounded(CHANNEL_CAPACITY);
        (Self { inner: tx }, rx)
    }

    /// Send a command to the render thread.
    ///
    /// Blocks if the channel is full (backpressure).
    /// Returns `Err(SendError)` if the render thread has exited.
    pub fn send(&self, cmd: RenderCommand) -> Result<(), SendError> {
        self.inner.send(cmd).map_err(|_| SendError)
    }
}

/// Error type for `CommandSender::send`.
#[derive(Debug)]
pub struct SendError;

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "render command send failed")
    }
}
