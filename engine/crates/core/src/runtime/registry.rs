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
    host_channel::{CriticalHostCommandSender, InputSendOutcome, InputStream},
    op_state::HostTx,
    payload_pool::PayloadPool,
    protocol::host_cmd::{GamepadState, HostCommand, TouchData},
    surface::{
        PublicSurfaceGeneration, SurfaceControl, SurfaceGeneration, SurfaceLease, SurfaceRef,
        SurfaceResourceLease,
    },
};

use crate::runtime::HostId;

/// Pending normal commands allowed per Host. Payload pools carry two extra
/// slots: one for a command held by the consumer after dequeue, and one for the
/// producer candidate inspected by queue coalescing or terminal supersession.
pub(crate) const HOST_NORMAL_COMMAND_CAPACITY: usize = 512;
pub(crate) const HOST_RELIABLE_INPUT_RESERVE: usize = 64;
const HOST_PAYLOAD_POOL_CAPACITY: usize =
    HOST_NORMAL_COMMAND_CAPACITY + HOST_RELIABLE_INPUT_RESERVE + 2;

/// Allocation-free error returned by a direct per-Session Host ingress.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostIngressSendError {
    Full,
    Closed,
}

/// Cloneable data-plane handles captured once after Host startup.
///
/// Calls through this value never acquire the global Host, VSync or debug-stats
/// registries. The stats handle is held here for that reason: it used to be looked up
/// by id on every event, which is a lock shared with every other Session on the
/// hottest path in the engine.
#[derive(Clone)]
pub struct HostIngress {
    host_id: HostId,
    tx: HostTx,
    vsync_tx: Option<crossbeam_channel::Sender<f64>>,
    touch_pool: PayloadPool<TouchData>,
    gamepad_pool: PayloadPool<GamepadState>,
    input_saturation_notified: Arc<AtomicBool>,
    stats: Arc<shared::stats::DebugStats>,
}

impl HostIngress {
    #[inline]
    pub const fn host_id(&self) -> HostId {
        self.host_id
    }

    /// Claim the adapter notification for the current input-saturation episode.
    ///
    /// All clones share this gate. A successful semantic input send rearms it.
    #[inline]
    pub fn claim_input_saturation_notification(&self) -> bool {
        self.input_saturation_notified
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    #[inline]
    pub fn try_send(&self, command: HostCommand) -> Result<(), HostIngressSendError> {
        match self.tx.try_send(command) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.stats.command_drops.fetch_add(1, Ordering::Relaxed);
                Err(HostIngressSendError::Full)
            }
            Err(TrySendError::Closed(_)) => Err(HostIngressSendError::Closed),
        }
    }

    /// Enqueue a touch batch in a preallocated payload slot.
    #[inline]
    pub fn try_send_touch(
        &self,
        touch: TouchData,
    ) -> Result<InputSendOutcome, HostIngressSendError> {
        let touch_type = touch.touch_type;
        let payload = self.touch_pool.try_insert(touch).map_err(|_| {
            self.record_input_saturation();
            HostIngressSendError::Full
        })?;
        let command = HostCommand::OnTouch(payload);
        let result = match touch_type {
            shared::protocol::host_cmd::TouchType::Move => {
                self.tx.try_send_coalescible(InputStream::Touch, command)
            }
            shared::protocol::host_cmd::TouchType::Start => {
                self.tx.try_send_reliable(Some(InputStream::Touch), command)
            }
            shared::protocol::host_cmd::TouchType::End
            | shared::protocol::host_cmd::TouchType::Cancel => {
                self.tx.try_send_terminal(Some(InputStream::Touch), command)
            }
        };
        self.map_input_result(result)
    }

    /// Enqueue one desktop pointer transition or motion sample.
    #[inline]
    pub fn try_send_pointer(
        &self,
        command: HostCommand,
    ) -> Result<InputSendOutcome, HostIngressSendError> {
        let result = match command {
            command @ HostCommand::OnMouseMove { .. } => {
                self.tx.try_send_coalescible(InputStream::Pointer, command)
            }
            command @ HostCommand::OnMouseDown { .. } => self
                .tx
                .try_send_reliable(Some(InputStream::Pointer), command),
            command @ HostCommand::OnMouseUp { .. } => self
                .tx
                .try_send_terminal(Some(InputStream::Pointer), command),
            command => self
                .tx
                .try_send(command)
                .map(|()| InputSendOutcome::Enqueued),
        };
        self.map_input_result(result)
    }

    /// Enqueue one physical keyboard transition.
    #[inline]
    pub fn try_send_key(
        &self,
        command: HostCommand,
    ) -> Result<InputSendOutcome, HostIngressSendError> {
        let result = match command {
            command @ HostCommand::OnKeyUp { .. } => self.tx.try_send_terminal(None, command),
            command => self
                .tx
                .try_send(command)
                .map(|()| InputSendOutcome::Enqueued),
        };
        self.map_input_result(result)
    }

    /// Enqueue one soft-keyboard event.
    #[inline]
    pub fn try_send_keyboard(
        &self,
        command: HostCommand,
    ) -> Result<InputSendOutcome, HostIngressSendError> {
        let result = match command {
            command @ HostCommand::OnKeyboardComplete { .. } => {
                self.tx.try_send_terminal(None, command)
            }
            command => self
                .tx
                .try_send(command)
                .map(|()| InputSendOutcome::Enqueued),
        };
        self.map_input_result(result)
    }

    /// Enqueue one IME composition transition or replaceable preedit state.
    #[inline]
    pub fn try_send_composition(
        &self,
        command: HostCommand,
    ) -> Result<InputSendOutcome, HostIngressSendError> {
        let result = match command {
            command @ HostCommand::OnCompositionUpdate { .. } => self
                .tx
                .try_send_coalescible(InputStream::Composition, command),
            command @ HostCommand::OnCompositionStart { .. } => self
                .tx
                .try_send_reliable(Some(InputStream::Composition), command),
            command @ HostCommand::OnCompositionEnd { .. } => self
                .tx
                .try_send_terminal(Some(InputStream::Composition), command),
            command => self
                .tx
                .try_send(command)
                .map(|()| InputSendOutcome::Enqueued),
        };
        self.map_input_result(result)
    }

    /// Enqueue one gamepad topology transition.
    #[inline]
    pub fn try_send_gamepad_connection(
        &self,
        command: HostCommand,
    ) -> Result<InputSendOutcome, HostIngressSendError> {
        let result = match command {
            command @ HostCommand::OnGamepadConnected { index, .. } => self
                .tx
                .try_send_reliable(Some(InputStream::Gamepad(index)), command),
            command @ HostCommand::OnGamepadDisconnected { index } => self
                .tx
                .try_send_terminal(Some(InputStream::Gamepad(index)), command),
            command => self
                .tx
                .try_send(command)
                .map(|()| InputSendOutcome::Enqueued),
        };
        self.map_input_result(result)
    }

    /// Enqueue a gamepad sample in a preallocated payload slot.
    #[inline]
    pub fn try_send_gamepad_state(
        &self,
        state: GamepadState,
    ) -> Result<InputSendOutcome, HostIngressSendError> {
        let stream = InputStream::Gamepad(state.index);
        let payload = self.gamepad_pool.try_insert(state).map_err(|_| {
            self.record_input_saturation();
            HostIngressSendError::Full
        })?;
        let result = self
            .tx
            .try_send_coalescible(stream, HostCommand::OnGamepadState(payload));
        self.map_input_result(result)
    }

    #[inline]
    fn map_input_result(
        &self,
        result: Result<InputSendOutcome, TrySendError<HostCommand>>,
    ) -> Result<InputSendOutcome, HostIngressSendError> {
        match result {
            Ok(outcome) => {
                self.input_saturation_notified
                    .store(false, Ordering::Release);
                match outcome {
                    InputSendOutcome::Enqueued => {}
                    InputSendOutcome::Coalesced => {
                        self.stats.input_coalesced.fetch_add(1, Ordering::Relaxed);
                    }
                    InputSendOutcome::Reserved => {
                        self.stats
                            .input_reliable_reserve_uses
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }
                Ok(outcome)
            }
            Err(TrySendError::Full(_)) => {
                self.record_input_saturation();
                Err(HostIngressSendError::Full)
            }
            Err(TrySendError::Closed(_)) => Err(HostIngressSendError::Closed),
        }
    }

    #[inline]
    fn record_input_saturation(&self) {
        self.stats.command_drops.fetch_add(1, Ordering::Relaxed);
        self.stats
            .input_saturation_events
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Deliver one externally paced frame timestamp. A Host without external
    /// pacing deliberately has no sender, making this a harmless no-op.
    #[inline]
    pub fn try_send_vsync(&self, frame_time_ms: f64) -> Result<(), HostIngressSendError> {
        let Some(tx) = &self.vsync_tx else {
            return Ok(());
        };
        match tx.try_send(frame_time_ms) {
            Ok(()) => Ok(()),
            Err(crossbeam_channel::TrySendError::Full(_)) => {
                self.stats.dropped_frames.fetch_add(1, Ordering::Relaxed);
                Err(HostIngressSendError::Full)
            }
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                Err(HostIngressSendError::Closed)
            }
        }
    }
}

static NEXT_HOST_ID: AtomicI32 = AtomicI32::new(1);

/// Per-host control handle published to platform callback threads.
///
/// The shutdown flag and Surface generation gate are queue-independent state:
/// a saturated command queue cannot swallow shutdown or keep a retired Surface
/// generation live.
pub(crate) struct HostHandle {
    tx: HostTx,
    critical_tx: CriticalHostCommandSender,
    surface_control: Arc<SurfaceControl>,
    touch_pool: PayloadPool<TouchData>,
    gamepad_pool: PayloadPool<GamepadState>,
    input_saturation_notified: Arc<AtomicBool>,
    stats: Arc<shared::stats::DebugStats>,
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
    surface_control: Arc<SurfaceControl>,
) -> Option<HostHandle> {
    // Resolved before the registry lock is taken. Acquiring one process-wide lock
    // while holding another is how a lock cycle starts, and these two are reached
    // from different threads at bring-up.
    let stats = shared::stats::stats_for(id);
    let mut map = host_senders().write();
    map.insert(
        id,
        HostHandle {
            tx,
            critical_tx,
            surface_control,
            touch_pool: PayloadPool::new(HOST_PAYLOAD_POOL_CAPACITY),
            gamepad_pool: PayloadPool::new(HOST_PAYLOAD_POOL_CAPACITY),
            input_saturation_notified: Arc::new(AtomicBool::new(false)),
            stats,
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
fn registered_surface_control(host_id: HostId) -> Result<Arc<SurfaceControl>, String> {
    {
        let map = host_senders().read();
        map.get(&host_id)
            .map(|handle| Arc::clone(&handle.surface_control))
    }
    .ok_or_else(|| {
        debug!("surface control: host_id={host_id} not found (likely already shut down)");
        format!("Cannot find host_id={host_id} Surface control")
    })
}

/// Capture direct data-plane handles after Host initialization completes.
pub fn host_ingress(host_id: HostId) -> Result<HostIngress, String> {
    let (tx, touch_pool, gamepad_pool, input_saturation_notified, stats) = {
        let map = host_senders().read();
        map.get(&host_id).map(|handle| {
            (
                handle.tx.clone(),
                handle.touch_pool.clone(),
                handle.gamepad_pool.clone(),
                Arc::clone(&handle.input_saturation_notified),
                Arc::clone(&handle.stats),
            )
        })
    }
    .ok_or_else(|| format!("Cannot find host_id={host_id} ingress"))?;

    Ok(HostIngress {
        host_id,
        tx,
        vsync_tx: crate::runtime::vsync::sender(host_id),
        touch_pool,
        gamepad_pool,
        input_saturation_notified,
        stats,
    })
}

fn issue_surface_token(host_id: HostId) -> Result<shared::surface::SurfaceLivenessToken, String> {
    let control = registered_surface_control(host_id)?;

    control.attach_or_update().map_err(|error| {
        warn!("lease_surface: host_id={host_id} failed: {error}");
        format!("Cannot lease Surface for host_id={host_id}: {error}")
    })
}

pub fn lease_surface(host_id: HostId, surface: SurfaceRef) -> Result<SurfaceLease, String> {
    let token = issue_surface_token(host_id)?;
    Ok(SurfaceLease::new(surface, token))
}

/// Start a resource lifetime carrying the embedding host's public generation.
pub fn lease_surface_tracked(
    host_id: HostId,
    surface: SurfaceRef,
    public_generation: PublicSurfaceGeneration,
) -> Result<SurfaceLease, String> {
    let token = issue_surface_token(host_id)?;
    Ok(SurfaceLease::new_tracked(surface, token, public_generation))
}

/// Build a same-attachment metrics update while retaining the original native
/// resource lifetime and public generation.
pub fn lease_surface_with_resource(
    host_id: HostId,
    surface: SurfaceRef,
    resource: SurfaceResourceLease,
) -> Result<SurfaceLease, String> {
    let token = issue_surface_token(host_id)?;
    SurfaceLease::with_resource(surface, token, resource).map_err(|error| {
        warn!("lease_surface_with_resource: host_id={host_id} failed: {error}");
        format!("Cannot update Surface for host_id={host_id}: {error}")
    })
}

/// Synchronously retire the registered Host's current Surface generation.
///
/// A successful `Some(generation)` is the queue-independent present barrier;
/// `None` means the generation was already retired and no duplicate lifecycle
/// command should be emitted.
pub fn retire_surface(host_id: HostId) -> Result<Option<SurfaceGeneration>, String> {
    Ok(registered_surface_control(host_id)?.retire_current_and_request())
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

/// Send a trusted asynchronous result that must not be silently dropped.
///
/// `send_command_to_host` is best-effort (`try_send`) and drops on a full queue.
/// Host-owned callback results share the ordered host channel but bypass its
/// normal-command quota, so an accepted platform result cannot disappear just
/// because input filled the data-plane budget. This capability is deliberately
/// held by the registry rather than exposed on `HostIngress`.
pub fn send_reliable_command_to_host(host_id: HostId, cmd: HostCommand) -> Result<(), String> {
    let sender = {
        let map = host_senders().read();
        map.get(&host_id).map(|handle| handle.critical_tx.clone())
    }
    .ok_or_else(|| {
        debug!(
            "send_reliable_command_to_host: host_id={host_id} not found (likely already shut down)"
        );
        format!("Cannot find host_id={host_id} sender")
    })?;

    match sender.send(cmd) {
        Ok(()) => Ok(()),
        Err(_error) => {
            let _ = unregister_sender(host_id);
            Err(format!(
                "Failed to send reliable command to host {host_id}: channel is closed"
            ))
        }
    }
}

/// Send a lifecycle/surface command that must not be silently dropped.
///
/// Lifecycle transitions use the same trusted reliable lane as host callback
/// results, preserving one FIFO across every non-droppable command class.
pub fn send_critical_command_to_host(host_id: HostId, cmd: HostCommand) -> Result<(), String> {
    send_reliable_command_to_host(host_id, cmd)
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
    let (sender, surface_control) = {
        let map = host_senders().read();
        let Some(handle) = map.get(&id) else {
            // Already unregistered => the host is gone => shutdown goal achieved.
            debug!("shutdown_host: host_id={id} not found (already shut down)");
            return Ok(());
        };
        (handle.tx.clone(), Arc::clone(&handle.surface_control))
    };
    // Queue-independent presentation barrier. This happens before the nudge
    // and before render join; late attach attempts observe shutdown and fail.
    surface_control.shutdown();
    // Best-effort nudge so a host parked on `recv()` reacts immediately; if the
    // normal budget is full this send is dropped, but the flag above still stops
    // the loop when it next iterates.
    let _ = sender.try_send(HostCommand::Shutdown);
    Ok(())
}

#[cfg(test)]
mod tests {
    use shared::host_channel::InputSendOutcome;
    use shared::surface::{Surface, SurfaceControl, SurfaceRef};
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
        let control = Arc::new(SurfaceControl::new());
        assert!(register_sender(id, tx, critical_tx, control).is_none());
        RegisteredHost(id)
    }

    struct RegisteredStats(HostId);

    impl Drop for RegisteredStats {
        fn drop(&mut self) {
            shared::stats::unregister_stats(self.0);
        }
    }

    fn test_ingress(
        normal_capacity: usize,
        reliable_capacity: usize,
    ) -> (
        HostIngress,
        shared::host_channel::CriticalHostCommandSender,
        shared::host_channel::HostCommandReceiver,
        Arc<shared::stats::DebugStats>,
        RegisteredStats,
    ) {
        let id = alloc_host_id();
        let stats = shared::stats::stats_for(id);
        let (tx, critical_tx, rx) =
            shared::host_channel::channel_with_reserve(normal_capacity, reliable_capacity);
        (
            HostIngress {
                host_id: id,
                tx,
                vsync_tx: None,
                touch_pool: PayloadPool::new(normal_capacity + reliable_capacity + 2),
                gamepad_pool: PayloadPool::new(normal_capacity + reliable_capacity + 2),
                input_saturation_notified: Arc::new(AtomicBool::new(false)),
                stats: Arc::clone(&stats),
            },
            critical_tx,
            rx,
            stats,
            RegisteredStats(id),
        )
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
        assert!(register_sender(id, tx, critical_tx, Arc::new(SurfaceControl::new()),).is_none());
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

    #[test]
    fn reliable_host_callback_bypasses_saturated_normal_budget() {
        let id = alloc_host_id();
        let (tx, critical_tx, mut rx) = shared::host_channel::channel(1);
        assert!(register_sender(id, tx, critical_tx, Arc::new(SurfaceControl::new()),).is_none());
        let _registration = RegisteredHost(id);

        send_command_to_host(id, HostCommand::Restart).unwrap();
        send_reliable_command_to_host(
            id,
            HostCommand::InvokeHostHook {
                hook: "_internalOnAuthorizeResult",
                args_json: "[]".into(),
            },
        )
        .unwrap();

        assert!(matches!(rx.try_recv(), Ok(HostCommand::Restart)));
        assert!(matches!(
            rx.try_recv(),
            Ok(HostCommand::InvokeHostHook {
                hook: "_internalOnAuthorizeResult",
                ..
            })
        ));
    }

    #[test]
    fn direct_touch_ingress_coalesces_moves_and_terminal_supersedes_them() {
        use shared::protocol::host_cmd::{TouchData, TouchPoint, TouchType};

        fn touch(touch_type: TouchType, timestamp_ms: i64) -> TouchData {
            TouchData {
                touch_type,
                count: 1,
                points: [TouchPoint::default(); 10],
                timestamp_ms,
            }
        }

        let (ingress, _critical_tx, mut rx, stats, _registered_stats) = test_ingress(1, 1);

        assert!(matches!(
            ingress.try_send_touch(touch(TouchType::Move, 1)),
            Ok(InputSendOutcome::Enqueued)
        ));
        assert!(matches!(
            ingress.try_send_touch(touch(TouchType::Move, 2)),
            Ok(InputSendOutcome::Coalesced)
        ));
        assert!(matches!(
            ingress.try_send_touch(touch(TouchType::End, 3)),
            Ok(InputSendOutcome::Enqueued)
        ));
        match rx.try_recv() {
            Ok(HostCommand::OnTouch(payload)) => {
                assert_eq!(payload.touch_type, TouchType::End);
                assert_eq!(payload.timestamp_ms, 3);
            }
            other => panic!("unexpected terminal command: {other:?}"),
        }
        assert!(rx.try_recv().is_err());
        assert_eq!(stats.input_coalesced.load(Ordering::Relaxed), 1);
        assert_eq!(stats.input_reliable_reserve_uses.load(Ordering::Relaxed), 0);
        assert_eq!(stats.input_saturation_events.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn terminal_touch_has_a_candidate_slot_at_peak_payload_occupancy() {
        use shared::protocol::host_cmd::{TouchData, TouchPoint, TouchType};

        fn touch(touch_type: TouchType, timestamp_ms: i64) -> TouchData {
            TouchData {
                touch_type,
                count: 1,
                points: [TouchPoint::default(); 10],
                timestamp_ms,
            }
        }

        let (ingress, _critical_tx, mut rx, stats, _registered_stats) = test_ingress(2, 1);
        ingress.try_send_touch(touch(TouchType::Move, 0)).unwrap();
        let held_by_consumer = match rx.try_recv().unwrap() {
            HostCommand::OnTouch(payload) => payload,
            command => panic!("unexpected command held by consumer: {command:?}"),
        };

        ingress.try_send_touch(touch(TouchType::Start, 1)).unwrap();
        ingress.try_send_touch(touch(TouchType::Move, 2)).unwrap();
        assert_eq!(
            ingress.try_send_touch(touch(TouchType::Start, 3)),
            Ok(InputSendOutcome::Reserved)
        );

        assert_eq!(
            ingress.try_send_touch(touch(TouchType::End, 4)),
            Ok(InputSendOutcome::Enqueued),
            "terminal input must acquire a candidate slot before superseding queued motion"
        );
        assert_eq!(stats.input_saturation_events.load(Ordering::Relaxed), 0);
        drop(held_by_consumer);
    }

    #[test]
    fn reliable_input_uses_reserve_and_reports_only_actual_refusal() {
        let (ingress, _critical_tx, _rx, stats, _registered_stats) = test_ingress(1, 1);
        ingress.try_send(HostCommand::Restart).unwrap();

        assert!(matches!(
            ingress.try_send_pointer(HostCommand::OnMouseDown {
                x: 1.0,
                y: 2.0,
                button: 0,
                timestamp_ms: 3.0,
            }),
            Ok(InputSendOutcome::Reserved)
        ));
        assert_eq!(
            ingress.try_send_pointer(HostCommand::OnMouseUp {
                x: 1.0,
                y: 2.0,
                button: 0,
                timestamp_ms: 4.0,
            }),
            Err(HostIngressSendError::Full)
        );
        assert_eq!(stats.input_reliable_reserve_uses.load(Ordering::Relaxed), 1);
        assert_eq!(stats.input_saturation_events.load(Ordering::Relaxed), 1);
        assert_eq!(stats.command_drops.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn composition_updates_coalesce_and_end_preserves_the_transition_pair() {
        let (ingress, _critical_tx, mut rx, stats, _registered_stats) = test_ingress(2, 1);
        ingress
            .try_send_composition(HostCommand::OnCompositionStart {
                data: String::new(),
            })
            .unwrap();
        ingress
            .try_send_composition(HostCommand::OnCompositionUpdate {
                data: "n".to_owned(),
            })
            .unwrap();
        assert_eq!(
            ingress.try_send_composition(HostCommand::OnCompositionUpdate {
                data: "ni".to_owned(),
            }),
            Ok(InputSendOutcome::Coalesced)
        );
        ingress
            .try_send_composition(HostCommand::OnCompositionEnd {
                data: "ni".to_owned(),
            })
            .unwrap();

        assert!(matches!(
            rx.try_recv(),
            Ok(HostCommand::OnCompositionStart { .. })
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(HostCommand::OnCompositionEnd { data }) if data == "ni"
        ));
        assert!(rx.try_recv().is_err());
        assert_eq!(stats.input_coalesced.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn key_up_and_keyboard_complete_each_use_the_reliable_reserve() {
        let key_up = HostCommand::OnKeyUp {
            key: "a".to_owned(),
            code: "KeyA".to_owned(),
            timestamp_ms: 1.0,
            modifiers: 0,
            repeat: false,
        };
        let (key_ingress, _critical_tx, _rx, key_stats, _registered_stats) = test_ingress(1, 1);
        key_ingress.try_send(HostCommand::Restart).unwrap();
        assert_eq!(
            key_ingress.try_send_key(key_up),
            Ok(InputSendOutcome::Reserved)
        );
        assert_eq!(
            key_stats
                .input_reliable_reserve_uses
                .load(Ordering::Relaxed),
            1
        );

        let (keyboard_ingress, _critical_tx, _rx, keyboard_stats, _registered_stats) =
            test_ingress(1, 1);
        keyboard_ingress.try_send(HostCommand::Restart).unwrap();
        assert_eq!(
            keyboard_ingress.try_send_keyboard(HostCommand::OnKeyboardComplete {
                value: "done".to_owned(),
            }),
            Ok(InputSendOutcome::Reserved)
        );
        assert_eq!(
            keyboard_stats
                .input_reliable_reserve_uses
                .load(Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn gamepad_samples_coalesce_by_index_and_disconnect_supersedes_its_sample() {
        use shared::protocol::host_cmd::{
            GAMEPAD_MAX_AXES, GAMEPAD_MAX_BUTTONS, GamepadButtonState, GamepadState,
        };

        fn state(index: u32, timestamp_ms: f64) -> GamepadState {
            GamepadState {
                index,
                axis_count: 0,
                button_count: 0,
                axes: [0.0; GAMEPAD_MAX_AXES],
                buttons: [GamepadButtonState::default(); GAMEPAD_MAX_BUTTONS],
                timestamp_ms,
            }
        }

        let (ingress, _critical_tx, mut rx, stats, _registered_stats) = test_ingress(3, 1);
        ingress.try_send_gamepad_state(state(0, 1.0)).unwrap();
        ingress.try_send_gamepad_state(state(1, 2.0)).unwrap();
        assert_eq!(
            ingress.try_send_gamepad_state(state(0, 3.0)),
            Ok(InputSendOutcome::Coalesced)
        );
        ingress
            .try_send_gamepad_connection(HostCommand::OnGamepadDisconnected { index: 0 })
            .unwrap();

        match rx.try_recv() {
            Ok(HostCommand::OnGamepadState(sample)) => {
                assert_eq!(sample.index, 1);
                assert_eq!(sample.timestamp_ms, 2.0);
            }
            other => panic!("unexpected gamepad sample: {other:?}"),
        }
        assert!(matches!(
            rx.try_recv(),
            Ok(HostCommand::OnGamepadDisconnected { index: 0 })
        ));
        assert!(rx.try_recv().is_err());
        assert_eq!(stats.input_coalesced.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn saturation_notification_is_shared_and_rearmed_by_success() {
        let (ingress, _critical_tx, mut rx, _stats, _registered_stats) = test_ingress(1, 1);
        let clone = ingress.clone();
        ingress.try_send(HostCommand::Restart).unwrap();
        ingress
            .try_send_pointer(HostCommand::OnMouseDown {
                x: 1.0,
                y: 2.0,
                button: 0,
                timestamp_ms: 2.0,
            })
            .unwrap();

        assert_eq!(
            ingress.try_send_pointer(HostCommand::OnMouseUp {
                x: 1.0,
                y: 2.0,
                button: 0,
                timestamp_ms: 3.0,
            }),
            Err(HostIngressSendError::Full)
        );
        assert!(ingress.claim_input_saturation_notification());
        assert!(!clone.claim_input_saturation_notification());

        rx.try_recv().unwrap();
        rx.try_recv().unwrap();
        ingress
            .try_send_pointer(HostCommand::OnMouseDown {
                x: 2.0,
                y: 3.0,
                button: 0,
                timestamp_ms: 4.0,
            })
            .unwrap();
        ingress
            .try_send_pointer(HostCommand::OnMouseDown {
                x: 2.0,
                y: 3.0,
                button: 0,
                timestamp_ms: 4.5,
            })
            .unwrap();
        assert_eq!(
            ingress.try_send_pointer(HostCommand::OnMouseUp {
                x: 2.0,
                y: 3.0,
                button: 0,
                timestamp_ms: 5.0,
            }),
            Err(HostIngressSendError::Full)
        );
        assert!(clone.claim_input_saturation_notification());
    }

    /// Section 7.3: no per-event path acquires a lock shared beyond its own session.
    ///
    /// One gate per shared lock rather than one holding both, so a failure names the
    /// registry the path reached for instead of leaving it to be guessed.
    mod cross_session_locks {
        use super::*;
        use migo_contention_probe::{PATIENCE, PerEventPath, assert_completes_while_locked};
        use shared::protocol::host_cmd::{TouchData, TouchPoint, TouchType};

        fn touch(touch_type: TouchType) -> TouchData {
            TouchData {
                touch_type,
                count: 1,
                points: [TouchPoint::default(); 10],
                timestamp_ms: 1,
            }
        }

        #[test]
        fn a_touch_send_does_not_reach_the_host_registry() {
            let (ingress, _critical_tx, _rx, stats, _registered_stats) = test_ingress(4, 2);

            let outcome = assert_completes_while_locked(
                PerEventPath {
                    path: "HostIngress::try_send_touch",
                    shared_lock: "runtime::registry HOST_SENDERS",
                    patience: PATIENCE,
                },
                host_senders(),
                move || {
                    let first = ingress.try_send_touch(touch(TouchType::Move));
                    let coalesced = ingress.try_send_touch(touch(TouchType::Move));
                    (first, coalesced)
                },
            );

            assert_eq!(outcome.0, Ok(InputSendOutcome::Enqueued));
            assert_eq!(outcome.1, Ok(InputSendOutcome::Coalesced));
            // Proof the send reached the accounting tail rather than returning early:
            // an operation that did nothing would satisfy the gate.
            assert_eq!(stats.input_coalesced.load(Ordering::Relaxed), 1);
        }

        #[test]
        fn a_touch_send_does_not_reach_the_stats_registry() {
            let (ingress, _critical_tx, _rx, stats, _registered_stats) = test_ingress(4, 2);

            let outcome = assert_completes_while_locked(
                PerEventPath {
                    path: "HostIngress::try_send_touch",
                    shared_lock: "shared::stats STATS",
                    patience: PATIENCE,
                },
                shared::stats::registry_lock_for_contention_probe(),
                move || {
                    let first = ingress.try_send_touch(touch(TouchType::Move));
                    let coalesced = ingress.try_send_touch(touch(TouchType::Move));
                    (first, coalesced)
                },
            );

            assert_eq!(outcome.0, Ok(InputSendOutcome::Enqueued));
            assert_eq!(outcome.1, Ok(InputSendOutcome::Coalesced));
            assert_eq!(stats.input_coalesced.load(Ordering::Relaxed), 1);
        }
    }
}
