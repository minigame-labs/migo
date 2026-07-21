//! Ordered host-command transport with bounded data traffic and a reliable
//! control path.
//!
//! Normal commands hold one semaphore permit while queued. Trusted lifecycle
//! commands skip that quota but use the same underlying FIFO, so they cannot be
//! displaced by untrusted traffic and cannot reorder across channel classes.

use std::sync::Arc;

use tokio::sync::{
    OwnedSemaphorePermit, Semaphore,
    mpsc::{
        UnboundedReceiver, UnboundedSender,
        error::{SendError, TrySendError},
    },
};

use crate::protocol::host_cmd::HostCommand;

struct QueuedHostCommand {
    command: HostCommand,
    _normal_permit: Option<OwnedSemaphorePermit>,
}

impl QueuedHostCommand {
    fn into_command(self) -> HostCommand {
        let Self {
            command,
            _normal_permit,
        } = self;
        command
    }
}

/// Cloneable normal-command sender shared by game-controlled producers.
///
/// Normal producers must not be able to bypass the pending-command budget:
///
/// ```compile_fail
/// use shared::{HostCommand, host_channel::channel};
///
/// let endpoints = channel(1);
/// endpoints
///     .0
///     .try_send_critical(HostCommand::OnHide)
///     .unwrap();
/// ```
///
/// The current control-plane method is unavailable as well:
///
/// ```compile_fail
/// use shared::{HostCommand, host_channel::channel};
///
/// let endpoints = channel(1);
/// endpoints.0.send(HostCommand::OnHide).unwrap();
/// ```
#[derive(Clone)]
pub struct HostCommandSender {
    tx: UnboundedSender<QueuedHostCommand>,
    normal_slots: Arc<Semaphore>,
}

/// Trusted control-plane capability held only by the host registry.
#[derive(Clone)]
pub struct CriticalHostCommandSender {
    tx: UnboundedSender<QueuedHostCommand>,
}

/// Sole consumer for the ordered host-command stream.
pub struct HostCommandReceiver {
    rx: UnboundedReceiver<QueuedHostCommand>,
}

/// Create an ordered host channel with a bounded normal-command budget.
pub fn channel(
    normal_capacity: usize,
) -> (
    HostCommandSender,
    CriticalHostCommandSender,
    HostCommandReceiver,
) {
    assert!(
        normal_capacity > 0,
        "host normal command capacity must be non-zero"
    );
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let normal_slots = Arc::new(Semaphore::new(normal_capacity));
    (
        HostCommandSender {
            tx: tx.clone(),
            normal_slots,
        },
        CriticalHostCommandSender { tx },
        HostCommandReceiver { rx },
    )
}

impl HostCommandSender {
    /// Enqueue a normal command without waiting, subject to the per-host quota.
    pub fn try_send(&self, command: HostCommand) -> Result<(), TrySendError<HostCommand>> {
        let permit = match self.normal_slots.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => return Err(TrySendError::Full(command)),
        };
        send_queued(&self.tx, command, Some(permit)).map_err(|error| TrySendError::Closed(error.0))
    }
}

impl CriticalHostCommandSender {
    /// Enqueue trusted control traffic without waiting or consuming normal
    /// command capacity.
    pub fn send(&self, command: HostCommand) -> Result<(), SendError<HostCommand>> {
        send_queued(&self.tx, command, None)
    }
}

fn send_queued(
    tx: &UnboundedSender<QueuedHostCommand>,
    command: HostCommand,
    normal_permit: Option<OwnedSemaphorePermit>,
) -> Result<(), SendError<HostCommand>> {
    tx.send(QueuedHostCommand {
        command,
        _normal_permit: normal_permit,
    })
    .map_err(|error| SendError(error.0.into_command()))
}

impl HostCommandReceiver {
    /// Receive the next command in the single cross-class FIFO order.
    pub async fn recv(&mut self) -> Option<HostCommand> {
        self.rx.recv().await.map(QueuedHostCommand::into_command)
    }

    /// Try to receive without waiting.
    pub fn try_recv(&mut self) -> Result<HostCommand, tokio::sync::mpsc::error::TryRecvError> {
        self.rx.try_recv().map(QueuedHostCommand::into_command)
    }
}

#[cfg(test)]
mod tests {
    use super::channel;
    use crate::protocol::host_cmd::HostCommand;
    use tokio::sync::mpsc::error::{SendError, TrySendError};

    #[test]
    fn critical_bypasses_full_normal_budget_and_keeps_fifo() {
        let (tx, critical_tx, mut rx) = channel(1);
        tx.try_send(HostCommand::Restart).unwrap();

        assert!(matches!(
            tx.try_send(HostCommand::Shutdown),
            Err(TrySendError::Full(HostCommand::Shutdown))
        ));
        critical_tx.send(HostCommand::OnHide).unwrap();

        assert!(matches!(rx.try_recv(), Ok(HostCommand::Restart)));
        assert!(matches!(rx.try_recv(), Ok(HostCommand::OnHide)));
    }

    #[test]
    fn receiving_normal_command_releases_its_budget() {
        let (tx, _critical_tx, mut rx) = channel(1);
        tx.try_send(HostCommand::Restart).unwrap();
        assert!(matches!(rx.try_recv(), Ok(HostCommand::Restart)));
        tx.try_send(HostCommand::Shutdown).unwrap();
        assert!(matches!(rx.try_recv(), Ok(HostCommand::Shutdown)));
    }

    #[test]
    fn critical_then_normal_preserves_fifo() {
        let (tx, critical_tx, mut rx) = channel(1);
        critical_tx.send(HostCommand::OnHide).unwrap();
        tx.try_send(HostCommand::Restart).unwrap();
        assert!(matches!(rx.try_recv(), Ok(HostCommand::OnHide)));
        assert!(matches!(rx.try_recv(), Ok(HostCommand::Restart)));
    }

    #[test]
    fn closed_receiver_returns_original_command() {
        let (tx, critical_tx, rx) = channel(1);
        drop(rx);
        assert!(matches!(
            tx.try_send(HostCommand::Restart),
            Err(TrySendError::Closed(HostCommand::Restart))
        ));
        assert!(matches!(
            critical_tx.send(HostCommand::OnHide),
            Err(SendError(HostCommand::OnHide))
        ));
    }
}
