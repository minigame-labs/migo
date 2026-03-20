use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

/// Runtime debug statistics collected by the render thread.
///
/// All fields are atomics for lock-free reads from the JNI polling thread.
#[derive(Default)]
pub struct DebugStats {
    /// Current FPS multiplied by 10 (e.g., 598 = 59.8 FPS).
    pub fps_x10: AtomicU32,
    /// Last frame time in microseconds.
    pub frame_time_us: AtomicU32,
    /// Total number of dropped RAF signals (cumulative).
    pub dropped_frames: AtomicU32,
    /// Fatal error code from the engine (0 = no error).
    /// Set when the host thread is terminated (e.g., OOM, Timeout).
    /// Java layer can poll this to detect engine errors.
    pub fatal_error_code: AtomicU32,
    /// Milliseconds from render thread start to first frame presentation.
    /// Set once on the first swap_buffers; remains 0 until then.
    pub first_frame_ms: AtomicU32,
    /// Total number of HostCommand messages dropped due to queue overflow (cumulative).
    /// Incremented by `send_command_to_host` when `try_send` returns `Full`.
    pub command_drops: AtomicU32,
}

impl DebugStats {
    #[inline]
    pub fn fps(&self) -> f32 {
        self.fps_x10.load(Ordering::Relaxed) as f32 / 10.0
    }

    #[inline]
    pub fn frame_time_ms(&self) -> f32 {
        self.frame_time_us.load(Ordering::Relaxed) as f32 / 1000.0
    }
}

static STATS: OnceLock<RwLock<HashMap<i32, Arc<DebugStats>>>> = OnceLock::new();

fn stats_map() -> &'static RwLock<HashMap<i32, Arc<DebugStats>>> {
    STATS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Register a new DebugStats for the given host_id. Returns the shared handle.
pub fn register_stats(id: i32) -> Arc<DebugStats> {
    let stats = Arc::new(DebugStats::default());
    stats_map()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, stats.clone());
    stats
}

/// Unregister stats for a host_id (cleanup on shutdown).
pub fn unregister_stats(id: i32) {
    stats_map()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id);
}

/// Get the DebugStats for a host_id (used by JNI polling).
pub fn get_stats(id: i32) -> Option<Arc<DebugStats>> {
    stats_map()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .cloned()
}
