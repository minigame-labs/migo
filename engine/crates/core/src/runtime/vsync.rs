//! Global registry for VSync signal senders.
//!
//! On Android, the Choreographer fires VSync callbacks on the UI thread.
//! The JNI callback looks up the sender for the given host_id and sends
//! the frame timestamp to the render thread.

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

/// Send the Choreographer frame timestamp to the render thread for the given host_id.
/// Called from the JNI Choreographer callback.
pub fn send_vsync(id: i32, frame_time_ms: f64) {
    if let Some(tx) = senders().read().get(&id) {
        if tx.try_send(frame_time_ms).is_err() {
            if let Some(stats) = shared::stats::get_stats(id) {
                stats
                    .dropped_frames
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}
