use std::{
    collections::HashMap,
    sync::{
        OnceLock, RwLock,
        atomic::{AtomicI32, Ordering},
    },
};

use tokio::sync::mpsc::{Sender, error::TrySendError};

use shared::protocol::host_cmd::HostCommand;

use crate::runtime::HostId;

static NEXT_HOST_ID: AtomicI32 = AtomicI32::new(1);

static HOST_SENDERS: OnceLock<RwLock<HashMap<HostId, Sender<HostCommand>>>> = OnceLock::new();

#[inline]
fn host_senders() -> &'static RwLock<HashMap<HostId, Sender<HostCommand>>> {
    HOST_SENDERS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub(crate) fn alloc_host_id() -> HostId {
    NEXT_HOST_ID.fetch_add(1, Ordering::Relaxed)
}

/// Register sender for a host.
/// Returns the previous sender if existed (should normally be None).
pub(crate) fn register_sender(id: HostId, tx: Sender<HostCommand>) -> Option<Sender<HostCommand>> {
    let mut map = host_senders().write().unwrap_or_else(|e| e.into_inner());
    map.insert(id, tx)
}

/// Unregister sender for a host.
/// Returns removed sender if existed.
pub(crate) fn unregister_sender(id: HostId) -> Option<Sender<HostCommand>> {
    let mut map = host_senders().write().unwrap_or_else(|e| e.into_inner());
    map.remove(&id)
}

pub fn send_command_to_host(host_id: HostId, cmd: HostCommand) -> Result<(), String> {
    // Clone sender and drop lock before sending (lower contention / avoids lock hazards).
    let sender = {
        let map = host_senders().read().unwrap_or_else(|e| e.into_inner());
        map.get(&host_id).cloned()
    }
    .ok_or_else(|| format!("Cannot find host_id={host_id} sender"))?;

    match sender.try_send(cmd) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_cmd)) => {
            if let Some(stats) = shared::stats::get_stats(host_id) {
                stats.command_drops.fetch_add(1, Ordering::Relaxed);
            }
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

pub fn shutdown_host(id: HostId) -> Result<(), String> {
    send_command_to_host(id, HostCommand::Shutdown)
}
