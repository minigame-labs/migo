use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering},
    },
};

use parking_lot::RwLock;
use tokio::sync::mpsc::error::TrySendError;
use tracing::{debug, warn};

use shared::{
    host_channel::CriticalHostCommandSender, op_state::HostTx, protocol::host_cmd::HostCommand,
};

use crate::runtime::HostId;

static NEXT_HOST_ID: AtomicI32 = AtomicI32::new(1);

/// Per-host control handle: the ordered command sender plus a shutdown flag.
/// The flag is the authoritative, queue-independent shutdown signal so a full
/// normal-command budget can never swallow a shutdown request.
type HostHandle = (HostTx, CriticalHostCommandSender, Arc<AtomicBool>);

static HOST_SENDERS: OnceLock<RwLock<HashMap<HostId, HostHandle>>> = OnceLock::new();

#[inline]
fn host_senders() -> &'static RwLock<HashMap<HostId, HostHandle>> {
    HOST_SENDERS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Per-host monotonic surface destroy-epoch, shared with the render thread.
///
/// Incremented by the JNI/UI thread on every `onSurfaceDestroyed` (before the
/// callback returns and Android abandons the BufferQueue). Each new surface
/// captures the counter value at `updateSurface` time (stored on the SurfaceRef
/// via `Surface::surface_epoch`). The render thread compares its current
/// surface's epoch against this live counter every frame; any mismatch means a
/// destroy occurred after that surface was handed off, so it stops presenting
/// immediately — synchronously, and independent of the (lossy, async) command
/// queue. A monotonic counter (not a boolean) is required so a fast
/// destroy->create->destroy can't be masked by an intervening value (ABA).
static DESTROY_EPOCHS: OnceLock<RwLock<HashMap<HostId, Arc<AtomicU64>>>> = OnceLock::new();

#[inline]
fn destroy_epochs() -> &'static RwLock<HashMap<HostId, Arc<AtomicU64>>> {
    DESTROY_EPOCHS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Register a host's destroy-epoch counter (created by the render service and
/// shared with the render thread). Returns the previous counter if any.
pub(crate) fn register_destroy_epoch(id: HostId, epoch: Arc<AtomicU64>) -> Option<Arc<AtomicU64>> {
    destroy_epochs().write().insert(id, epoch)
}

/// Remove a host's destroy-epoch counter (on shutdown).
pub(crate) fn unregister_destroy_epoch(id: HostId) -> Option<Arc<AtomicU64>> {
    destroy_epochs().write().remove(&id)
}

/// Advance a host's destroy-epoch. Called from JNI on `onSurfaceDestroyed` so
/// the render thread stops presenting to the surface being torn down. No-op if
/// the host has no registered counter yet.
pub fn bump_destroy_epoch(host_id: HostId) {
    if let Some(epoch) = destroy_epochs().read().get(&host_id) {
        epoch.fetch_add(1, Ordering::AcqRel);
    }
}

/// Read a host's current destroy-epoch. Called from JNI on `updateSurface` to
/// stamp the new surface with the epoch it corresponds to. Returns 0 if the
/// host has no registered counter yet (matches the render thread's init value).
pub fn current_destroy_epoch(host_id: HostId) -> u64 {
    destroy_epochs()
        .read()
        .get(&host_id)
        .map(|e| e.load(Ordering::Acquire))
        .unwrap_or(0)
}

pub(crate) fn alloc_host_id() -> HostId {
    NEXT_HOST_ID.fetch_add(1, Ordering::Relaxed)
}

/// Register sender for a host.
/// Returns the previous sender if existed (should normally be None).
pub(crate) fn register_sender(
    id: HostId,
    tx: HostTx,
    critical_tx: CriticalHostCommandSender,
    shutdown: Arc<AtomicBool>,
) -> Option<HostHandle> {
    let mut map = host_senders().write();
    map.insert(id, (tx, critical_tx, shutdown))
}

/// Unregister sender for a host.
/// Returns removed sender if existed.
pub(crate) fn unregister_sender(id: HostId) -> Option<HostHandle> {
    let mut map = host_senders().write();
    map.remove(&id)
}

pub fn send_command_to_host(host_id: HostId, cmd: HostCommand) -> Result<(), String> {
    // Clone sender and drop lock before sending (lower contention / avoids lock hazards).
    let sender = {
        let map = host_senders().read();
        map.get(&host_id).map(|(tx, _, _)| tx.clone())
    }
    .ok_or_else(|| {
        // Use debug level: this commonly happens during shutdown when JNI callbacks
        // (onVsync, touch, etc.) race with host thread exit + unregister.  Those
        // late arrivals are harmless and expected, so avoid noisy error logs.
        debug!("send_command_to_host: host_id={host_id} not found (likely already shut down)");
        format!("Cannot find host_id={host_id} sender")
    })?;

    match sender.try_send(cmd) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_cmd)) => {
            if let Some(stats) = shared::stats::get_stats(host_id) {
                stats.command_drops.fetch_add(1, Ordering::Relaxed);
            }
            warn!("Host {} command queue full, dropping command", host_id);
            Err(format!(
                "Failed to send command to host {host_id}: queue is full"
            ))
        }
        Err(TrySendError::Closed(_cmd)) => {
            // host likely dead -> cleanup registry entry
            let _ = unregister_sender(host_id);
            Err(format!(
                "Failed to send command to host {host_id}: channel is closed"
            ))
        }
    }
}

/// Send a lifecycle/surface command that must not be silently dropped.
///
/// `send_command_to_host` is best-effort (`try_send`) and drops on a full queue.
/// That is fine for high-frequency, coalescible commands (touch, vsync) but not
/// for surface/lifecycle transitions (UpdateSurface, SurfaceDestroyed, OnShow,
/// OnHide): dropping one permanently desyncs Java's lifecycle state from the
/// host/render/JS state. Critical commands share the ordered host channel but
/// bypass its normal-command quota, so enqueue never waits for normal backlog
/// capacity.
pub fn send_critical_command_to_host(host_id: HostId, cmd: HostCommand) -> Result<(), String> {
    let sender = {
        let map = host_senders().read();
        map.get(&host_id)
            .map(|(_, critical_tx, _)| critical_tx.clone())
    }
    .ok_or_else(|| {
        debug!(
            "send_critical_command_to_host: host_id={host_id} not found (likely already shut down)"
        );
        format!("Cannot find host_id={host_id} sender")
    })?;

    match sender.send(cmd) {
        Ok(()) => Ok(()),
        Err(_error) => {
            let _ = unregister_sender(host_id);
            Err(format!(
                "Failed to send critical command to host {host_id}: channel is closed"
            ))
        }
    }
}

pub fn shutdown_host(id: HostId) -> Result<(), String> {
    // Set the shutdown flag first: the host loop checks it every iteration, which
    // decouples shutdown from the command queue -- a full queue can no longer
    // swallow the request (the bug this fixes: HostCommand::Shutdown dropped by
    // try_send when the 512-slot normal budget is full, leaking the host thread).
    // The flag takes effect the next time the loop returns to the top of its iteration; it
    // does not preempt a runaway synchronous JS section that never yields (an
    // inherent bound of the cooperative loop; the v8-limits ANR watchdog covers
    // that case), so this is not an instantaneous hard kill.
    let sender = {
        let map = host_senders().read();
        let Some((tx, _, shutdown)) = map.get(&id) else {
            // Already unregistered => the host is gone => shutdown goal achieved.
            debug!("shutdown_host: host_id={id} not found (already shut down)");
            return Ok(());
        };
        shutdown.store(true, Ordering::Release);
        tx.clone()
    };
    // Best-effort nudge so a host parked on `recv()` reacts immediately; if the
    // normal budget is full this send is dropped, but the flag above still stops
    // the loop when it next iterates.
    let _ = sender.try_send(HostCommand::Shutdown);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RegisteredHost(HostId);

    impl Drop for RegisteredHost {
        fn drop(&mut self) {
            unregister_sender(self.0);
        }
    }

    #[test]
    fn critical_commands_bypass_saturated_normal_budget_in_fifo_order() {
        let id = alloc_host_id();
        let (tx, critical_tx, mut rx) = shared::host_channel::channel(1);
        assert!(register_sender(id, tx, critical_tx, Arc::new(AtomicBool::new(false))).is_none());
        let _registration = RegisteredHost(id);

        send_command_to_host(id, HostCommand::Restart).unwrap();
        assert!(send_command_to_host(id, HostCommand::Shutdown).is_err());
        send_critical_command_to_host(id, HostCommand::OnHide).unwrap();
        send_critical_command_to_host(id, HostCommand::OnShow { options_json: None }).unwrap();

        assert!(matches!(rx.try_recv(), Ok(HostCommand::Restart)));
        assert!(matches!(rx.try_recv(), Ok(HostCommand::OnHide)));
        assert!(matches!(
            rx.try_recv(),
            Ok(HostCommand::OnShow { options_json: None })
        ));
    }
}
