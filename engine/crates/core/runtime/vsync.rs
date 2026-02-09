//! Global registry for VSync signal senders.
//!
//! On Android, the Choreographer fires VSync callbacks on the UI thread.
//! The JNI callback looks up the sender for the given host_id and sends
//! the frame timestamp to the render thread.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use crossbeam_channel::Sender;

static VSYNC_SENDERS: OnceLock<RwLock<HashMap<i32, Sender<f64>>>> = OnceLock::new();

fn senders() -> &'static RwLock<HashMap<i32, Sender<f64>>> {
    VSYNC_SENDERS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Register a VSync sender for the given host_id.
pub fn register_vsync_sender(id: i32, tx: Sender<f64>) {
    senders()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, tx);
}

/// Unregister the VSync sender (called on host shutdown).
pub fn unregister_vsync_sender(id: i32) {
    senders()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id);
}

/// Send a VSync signal to the render thread for the given host_id.
/// Called from the JNI Choreographer callback.
pub fn send_vsync(id: i32, ts_ms: f64) {
    if let Some(tx) = senders()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
    {
        let _ = tx.try_send(ts_ms);
    }
}
