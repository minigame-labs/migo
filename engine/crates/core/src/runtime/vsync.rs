//! Global registry for VSync signal senders.
//!
//! On Android, the Choreographer fires VSync callbacks on the UI thread. The sender
//! for a host is cloned out of here once, at attach time, and the per-frame send goes
//! through that clone — see `HostIngress::try_send_vsync`. Nothing in this module is
//! on a frame path, which is the point: a per-frame lookup here would be a lock
//! shared with every other Session on every frame, which Section 7.3 forbids.

use std::collections::HashMap;
use std::sync::OnceLock;

use crossbeam_channel::Sender;
use parking_lot::RwLock;

static VSYNC_SENDERS: OnceLock<RwLock<HashMap<i32, Sender<f64>>>> = OnceLock::new();

fn senders() -> &'static RwLock<HashMap<i32, Sender<f64>>> {
    VSYNC_SENDERS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Clone one registered sender for a long-lived direct platform ingress.
/// Registry lookup is intentionally a cold attach-time operation.
pub(crate) fn sender(id: i32) -> Option<Sender<f64>> {
    senders().read().get(&id).cloned()
}

/// Register a VSync sender for the given host_id.
pub fn register_vsync_sender(id: i32, tx: Sender<f64>) {
    senders().write().insert(id, tx);
}

/// Unregister the VSync sender (called on host shutdown).
pub fn unregister_vsync_sender(id: i32) {
    senders().write().remove(&id);
}
