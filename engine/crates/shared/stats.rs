use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

use parking_lot::RwLock;

pub struct RenderMetricsSnapshot {
    pub fps_x10: u32,
    pub frame_time_us: u32,
    pub dropped_frames: u32,
    pub fatal_error_code: u32,
    pub first_frame_ms: u32,
    pub command_drops: u32,
    pub raf_latency_us: u32,
    pub swap_block_us: u32,
    pub upload_queue_depth: u32,
    pub glyph_atlas_miss: u32,
    // ---- render optimization metrics (appended at tail) ----
    pub partial_damage_frames: u32,
    pub full_surface_frames: u32,
    pub damage_area_k_pixels: u32,
    pub upload_frame_rejections: u32,
    pub dropped_upload_recoveries: u32,
}

impl RenderMetricsSnapshot {
    /// Magic bytes: 'M' 'G' (0x4D47) — identifies this as a Migo stats packet.
    pub const MAGIC: u16 = 0x4D47;
    /// Protocol version. Increment when field layout changes.
    pub const VERSION: u16 = 2;
    /// 4-byte header (2 magic + 2 version) + 60 bytes payload = 64.
    pub const HEADER_LEN: usize = 4;
    pub const PAYLOAD_LEN: usize = 60;
    pub const BYTE_LEN: usize = Self::HEADER_LEN + Self::PAYLOAD_LEN; // 64

    pub fn as_le_bytes(&self) -> [u8; Self::BYTE_LEN] {
        let mut bytes = [0u8; Self::BYTE_LEN];
        // Header
        bytes[0..2].copy_from_slice(&Self::MAGIC.to_le_bytes());
        bytes[2..4].copy_from_slice(&Self::VERSION.to_le_bytes());
        // Payload (offsets shifted by HEADER_LEN = 4)
        bytes[4..8].copy_from_slice(&self.fps_x10.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.frame_time_us.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.dropped_frames.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.fatal_error_code.to_le_bytes());
        bytes[20..24].copy_from_slice(&self.first_frame_ms.to_le_bytes());
        bytes[24..28].copy_from_slice(&self.command_drops.to_le_bytes());
        bytes[28..32].copy_from_slice(&self.raf_latency_us.to_le_bytes());
        bytes[32..36].copy_from_slice(&self.swap_block_us.to_le_bytes());
        bytes[36..40].copy_from_slice(&self.upload_queue_depth.to_le_bytes());
        bytes[40..44].copy_from_slice(&self.glyph_atlas_miss.to_le_bytes());
        bytes[44..48].copy_from_slice(&self.partial_damage_frames.to_le_bytes());
        bytes[48..52].copy_from_slice(&self.full_surface_frames.to_le_bytes());
        bytes[52..56].copy_from_slice(&self.damage_area_k_pixels.to_le_bytes());
        bytes[56..60].copy_from_slice(&self.upload_frame_rejections.to_le_bytes());
        bytes[60..64].copy_from_slice(&self.dropped_upload_recoveries.to_le_bytes());
        bytes
    }
}

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
    /// Last measured RAF scheduling latency in microseconds.
    pub raf_latency_us: AtomicU32,
    /// Last measured swap/present blocking time in microseconds.
    pub swap_block_us: AtomicU32,
    /// Current upload queue depth sampled by render diagnostics.
    pub upload_queue_depth: AtomicU32,
    /// Cumulative glyph atlas cache misses observed by text rendering.
    pub glyph_atlas_miss: AtomicU32,
    /// High-water mark of the audio command queue depth (peak pending items).
    /// Currently a placeholder — wiring to actual sender is deferred.
    pub audio_queue_hwm: AtomicU32,
    /// High-water mark of the IO command queue depth (peak pending items).
    /// Currently a placeholder — wiring to actual sender is deferred.
    pub io_queue_hwm: AtomicU32,
    // ---- Render optimization metrics ----
    /// Cumulative frames where damage resolved to Partial.
    pub partial_damage_frames: AtomicU32,
    /// Cumulative frames where damage resolved to FullSurface.
    pub full_surface_frames: AtomicU32,
    /// Cumulative partial damage area in 1000-pixel units (kpx).
    pub damage_area_k_pixels: AtomicU32,
    /// Cumulative per-frame upload budget rejections.
    pub upload_frame_rejections: AtomicU32,
    /// Cumulative dropped upload recoveries.
    pub dropped_upload_recoveries: AtomicU32,

    // ---- P11 render diagnostics (not in fixed snapshot) ----
    // These counters are NOT serialised into `RenderMetricsSnapshot`
    // to keep the existing Java ByteBuffer layout stable.  They're
    // exposed through a separate debug API (see
    // `engine/crates/graphics/render_diagnostics.rs`).  Bumping the
    // snapshot version to add them can happen in a later commit
    // once the Android consumer is updated in lock-step.

    /// Cumulative WebGL / Canvas2D draw calls dispatched.
    /// Canvas2D draws are counted once per paint op; WebGL draws are
    /// counted at each `glDrawArrays{Instanced}` /
    /// `glDrawElements{Instanced}` dispatch.
    pub draw_calls: AtomicU32,
    /// Cumulative GL state change calls issued (post-dedup).  A
    /// high ratio of `state_changes / draw_calls` indicates the
    /// game is thrashing state between draws; a very low one means
    /// the dedup is catching most redundancy.
    pub state_changes: AtomicU32,
    /// Cumulative bytes uploaded via PBO / direct `glTexSubImage2D`
    /// calls this frame.  Resets to 0 at each `Present`.
    pub texture_upload_bytes: AtomicU32,
    /// Cumulative measureText cache hits.  Paired with
    /// `measure_text_misses` gives the hit rate.
    pub measure_text_hits: AtomicU32,
    pub measure_text_misses: AtomicU32,
    /// Cumulative Skia shape-cache hits (SkTextBlob path cache).
    pub shape_cache_hits: AtomicU32,
    pub shape_cache_misses: AtomicU32,
    /// Cumulative Skia SkImage wrapper cache hits.
    pub sk_image_wrapper_hits: AtomicU32,
    pub sk_image_wrapper_misses: AtomicU32,
    /// Cumulative `GrDirectContext::reset()` calls (lazy reset
    /// path).  A fast-rising counter indicates frequent WebGL↔
    /// Canvas2D boundary crossings.
    pub skia_context_resets: AtomicU32,
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

    pub fn snapshot(&self) -> [u8; RenderMetricsSnapshot::BYTE_LEN] {
        RenderMetricsSnapshot {
            fps_x10: self.fps_x10.load(Ordering::Relaxed),
            frame_time_us: self.frame_time_us.load(Ordering::Relaxed),
            dropped_frames: self.dropped_frames.load(Ordering::Relaxed),
            fatal_error_code: self.fatal_error_code.load(Ordering::Relaxed),
            first_frame_ms: self.first_frame_ms.load(Ordering::Relaxed),
            command_drops: self.command_drops.load(Ordering::Relaxed),
            raf_latency_us: self.raf_latency_us.load(Ordering::Relaxed),
            swap_block_us: self.swap_block_us.load(Ordering::Relaxed),
            upload_queue_depth: self.upload_queue_depth.load(Ordering::Relaxed),
            glyph_atlas_miss: self.glyph_atlas_miss.load(Ordering::Relaxed),
            partial_damage_frames: self.partial_damage_frames.load(Ordering::Relaxed),
            full_surface_frames: self.full_surface_frames.load(Ordering::Relaxed),
            damage_area_k_pixels: self.damage_area_k_pixels.load(Ordering::Relaxed),
            upload_frame_rejections: self.upload_frame_rejections.load(Ordering::Relaxed),
            dropped_upload_recoveries: self.dropped_upload_recoveries.load(Ordering::Relaxed),
        }
        .as_le_bytes()
    }
}

static STATS: OnceLock<RwLock<HashMap<i32, Arc<DebugStats>>>> = OnceLock::new();

fn stats_map() -> &'static RwLock<HashMap<i32, Arc<DebugStats>>> {
    STATS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Register a new DebugStats for the given host_id. Returns the shared handle.
pub fn register_stats(id: i32) -> Arc<DebugStats> {
    let stats = Arc::new(DebugStats::default());
    stats_map().write().insert(id, stats.clone());
    stats
}

/// Unregister stats for a host_id (cleanup on shutdown).
pub fn unregister_stats(id: i32) {
    stats_map().write().remove(&id);
}

/// Get the DebugStats for a host_id (used by JNI polling).
pub fn get_stats(id: i32) -> Option<Arc<DebugStats>> {
    stats_map().read().get(&id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn render_metrics_snapshot_serializes_le_bytes() {
        let stats = DebugStats::default();
        stats.fps_x10.store(600, Ordering::Relaxed);
        stats.frame_time_us.store(16_600, Ordering::Relaxed);
        stats.first_frame_ms.store(320, Ordering::Relaxed);
        stats.command_drops.store(3, Ordering::Relaxed);
        stats.raf_latency_us.store(777, Ordering::Relaxed);
        stats.swap_block_us.store(888, Ordering::Relaxed);
        stats.upload_queue_depth.store(9, Ordering::Relaxed);
        stats.glyph_atlas_miss.store(10, Ordering::Relaxed);

        let bytes = stats.snapshot();

        // Total length is now 64 (4 header + 60 payload).
        assert_eq!(bytes.len(), RenderMetricsSnapshot::BYTE_LEN);
        assert_eq!(RenderMetricsSnapshot::BYTE_LEN, 64);

        // Header: magic 'MG' (0x4D47) at [0..2], version 2 at [2..4].
        assert_eq!(u16::from_le_bytes(bytes[0..2].try_into().unwrap()), RenderMetricsSnapshot::MAGIC);
        assert_eq!(u16::from_le_bytes(bytes[2..4].try_into().unwrap()), RenderMetricsSnapshot::VERSION);

        // Payload fields — all offsets shifted by +4 (HEADER_LEN).
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 600);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 16_600);
        assert_eq!(u32::from_le_bytes(bytes[20..24].try_into().unwrap()), 320);
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(bytes[28..32].try_into().unwrap()), 777);
        assert_eq!(u32::from_le_bytes(bytes[32..36].try_into().unwrap()), 888);
        assert_eq!(u32::from_le_bytes(bytes[36..40].try_into().unwrap()), 9);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 10);
        // Render optimization fields default to 0 at offsets 44-63.
        assert_eq!(u32::from_le_bytes(bytes[44..48].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(bytes[48..52].try_into().unwrap()), 0);
    }

    #[test]
    fn new_render_optimization_fields_serialize_at_tail() {
        let stats = DebugStats::default();
        stats.partial_damage_frames.store(42, Ordering::Relaxed);
        stats.full_surface_frames.store(7, Ordering::Relaxed);
        stats.damage_area_k_pixels.store(1500, Ordering::Relaxed);
        stats.upload_frame_rejections.store(3, Ordering::Relaxed);
        stats.dropped_upload_recoveries.store(1, Ordering::Relaxed);

        let bytes = stats.snapshot();

        assert_eq!(bytes.len(), 64);
        // Payload offsets shifted by +4 (HEADER_LEN).
        assert_eq!(u32::from_le_bytes(bytes[44..48].try_into().unwrap()), 42);
        assert_eq!(u32::from_le_bytes(bytes[48..52].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(bytes[52..56].try_into().unwrap()), 1500);
        assert_eq!(u32::from_le_bytes(bytes[56..60].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(bytes[60..64].try_into().unwrap()), 1);
    }
}
