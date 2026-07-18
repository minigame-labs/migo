use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
};

use parking_lot::RwLock;
use tokio::sync::mpsc::error::TrySendError;
use tracing::{debug, warn};

use shared::{
    host_channel::CriticalHostCommandSender,
    op_state::HostTx,
    protocol::host_cmd::HostCommand,
    surface::{SurfaceGeneration, SurfaceGenerationGate, SurfaceLease, SurfaceRef},
};

use crate::runtime::HostId;

static NEXT_HOST_ID: AtomicI32 = AtomicI32::new(1);

/// Per-host control handle published to platform callback threads.
///
/// The shutdown flag and Surface generation gate are queue-independent state:
/// a saturated command queue cannot swallow shutdown or keep a retired Surface
/// generation live.
pub(crate) struct HostHandle {
    tx: HostTx,
    critical_tx: CriticalHostCommandSender,
    shutdown: Arc<AtomicBool>,
    surface_gate: Arc<SurfaceGenerationGate>,
}

static HOST_SENDERS: OnceLock<RwLock<HashMap<HostId, HostHandle>>> = OnceLock::new();

#[inline]
fn host_senders() -> &'static RwLock<HashMap<HostId, HostHandle>> {
    HOST_SENDERS.get_or_init(|| RwLock::new(HashMap::new()))
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
    surface_gate: Arc<SurfaceGenerationGate>,
) -> Option<HostHandle> {
    let mut map = host_senders().write();
    map.insert(
        id,
        HostHandle {
            tx,
            critical_tx,
            shutdown,
            surface_gate,
        },
    )
}

/// Unregister sender for a host.
/// Returns removed sender if existed.
pub(crate) fn unregister_sender(id: HostId) -> Option<HostHandle> {
    let mut map = host_senders().write();
    map.remove(&id)
}

/// Pair a candidate Surface with the registered Host's current live
/// generation. The registry lock is released before the lock-free gate
/// transition and before the Surface lease is constructed.
pub fn lease_surface(host_id: HostId, surface: SurfaceRef) -> Result<SurfaceLease, String> {
    let (gate, shutdown) = {
        let map = host_senders().read();
        map.get(&host_id).map(|handle| {
            (
                Arc::clone(&handle.surface_gate),
                Arc::clone(&handle.shutdown),
            )
        })
    }
    .ok_or_else(|| {
        debug!("lease_surface: host_id={host_id} not found (likely already shut down)");
        format!("Cannot find host_id={host_id} Surface generation gate")
    })?;

    if shutdown.load(Ordering::Acquire) {
        return Err(format!(
            "Cannot lease Surface for host_id={host_id}: host is shutting down"
        ));
    }

    let token = gate.attach_or_update().map_err(|error| {
        warn!("lease_surface: host_id={host_id} failed: {error}");
        format!("Cannot lease Surface for host_id={host_id}: {error}")
    })?;

    // Close the race where shutdown begins after the first check but before
    // attach_or_update. The candidate has not escaped yet, and retiring the
    // current generation is correct for every concurrent shutdown contender.
    if shutdown.load(Ordering::Acquire) {
        gate.retire_current();
        return Err(format!(
            "Cannot lease Surface for host_id={host_id}: host is shutting down"
        ));
    }

    Ok(SurfaceLease::new(surface, token))
}

/// Synchronously retire the registered Host's current Surface generation.
///
/// A successful `Some(generation)` is the queue-independent present barrier;
/// `None` means the generation was already retired and no duplicate lifecycle
/// command should be emitted.
pub fn retire_surface(host_id: HostId) -> Result<Option<SurfaceGeneration>, String> {
    let gate = {
        let map = host_senders().read();
        map.get(&host_id)
            .map(|handle| Arc::clone(&handle.surface_gate))
    }
    .ok_or_else(|| {
        debug!("retire_surface: host_id={host_id} not found (likely already shut down)");
        format!("Cannot find host_id={host_id} Surface generation gate")
    })?;

    Ok(gate.retire_current())
}

pub fn send_command_to_host(host_id: HostId, cmd: HostCommand) -> Result<(), String> {
    // Clone sender and drop lock before sending (lower contention / avoids lock hazards).
    let sender = {
        let map = host_senders().read();
        map.get(&host_id).map(|handle| handle.tx.clone())
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
        map.get(&host_id).map(|handle| handle.critical_tx.clone())
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
    let (sender, surface_gate) = {
        let map = host_senders().read();
        let Some(handle) = map.get(&id) else {
            // Already unregistered => the host is gone => shutdown goal achieved.
            debug!("shutdown_host: host_id={id} not found (already shut down)");
            return Ok(());
        };
        handle.shutdown.store(true, Ordering::Release);
        (handle.tx.clone(), Arc::clone(&handle.surface_gate))
    };
    // Queue-independent presentation barrier. This happens before the nudge
    // and before render join; late attach attempts observe shutdown and fail.
    surface_gate.retire_current();
    // Best-effort nudge so a host parked on `recv()` reacts immediately; if the
    // normal budget is full this send is dropped, but the flag above still stops
    // the loop when it next iterates.
    let _ = sender.try_send(HostCommand::Shutdown);
    Ok(())
}

#[cfg(test)]
mod tests {
    use shared::surface::{Surface, SurfaceGenerationGate, SurfaceRef};
    use std::sync::atomic::AtomicUsize;

    use super::*;

    #[derive(Debug)]
    struct TestSurface {
        drops: Arc<AtomicUsize>,
    }

    impl Surface for TestSurface {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn size(&self) -> (u32, u32) {
            (640, 480)
        }
    }

    impl Drop for TestSurface {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn test_surface(drops: &Arc<AtomicUsize>) -> SurfaceRef {
        Arc::new(TestSurface {
            drops: Arc::clone(drops),
        })
    }

    struct RegisteredHost(HostId);

    impl Drop for RegisteredHost {
        fn drop(&mut self) {
            unregister_sender(self.0);
        }
    }

    fn register_test_host(id: HostId) -> RegisteredHost {
        let (tx, critical_tx, _rx) = shared::host_channel::channel(1);
        let gate = Arc::new(SurfaceGenerationGate::new());
        assert!(
            register_sender(id, tx, critical_tx, Arc::new(AtomicBool::new(false)), gate,).is_none()
        );
        RegisteredHost(id)
    }

    #[test]
    fn surface_lifecycle_is_scoped_to_the_registered_host() {
        let id = alloc_host_id();
        let _registration = register_test_host(id);
        let drops = Arc::new(AtomicUsize::new(0));

        let first = lease_surface(id, test_surface(&drops)).unwrap();
        assert_eq!(first.generation().get(), 1);
        assert!(first.is_live());

        assert_eq!(retire_surface(id).unwrap(), Some(first.generation()));
        assert!(!first.is_live());
        assert_eq!(retire_surface(id).unwrap(), None);

        let second = lease_surface(id, test_surface(&drops)).unwrap();
        assert_eq!(second.generation().get(), 2);
        assert!(second.is_live());
        assert!(!first.is_live());

        drop(first);
        drop(second);
        assert_eq!(drops.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn missing_host_rejects_and_drops_the_candidate_surface() {
        let id = alloc_host_id();
        let drops = Arc::new(AtomicUsize::new(0));

        assert!(lease_surface(id, test_surface(&drops)).is_err());
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert!(retire_surface(id).is_err());
    }

    #[test]
    fn shutdown_retires_the_surface_before_the_queue_nudge() {
        let id = alloc_host_id();
        let _registration = register_test_host(id);
        let drops = Arc::new(AtomicUsize::new(0));
        let lease = lease_surface(id, test_surface(&drops)).unwrap();

        shutdown_host(id).unwrap();

        assert!(!lease.is_live());
        assert_eq!(retire_surface(id).unwrap(), None);
        assert!(lease_surface(id, test_surface(&drops)).is_err());
        assert_eq!(drops.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn critical_commands_bypass_saturated_normal_budget_in_fifo_order() {
        let id = alloc_host_id();
        let (tx, critical_tx, mut rx) = shared::host_channel::channel(1);
        assert!(
            register_sender(
                id,
                tx,
                critical_tx,
                Arc::new(AtomicBool::new(false)),
                Arc::new(SurfaceGenerationGate::new()),
            )
            .is_none()
        );
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
