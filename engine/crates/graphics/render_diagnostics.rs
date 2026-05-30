//! Render-thread diagnostic counter sink.
//!
//! A thin wrapper over [`shared::stats::DebugStats`] that lets hot
//! paths bump counters without threading the stats handle through
//! every call site.  Idiomatic use:
//!
//! ```ignore
//! // Once, at render thread start:
//! render_diagnostics::install(stats_arc);
//!
//! // Anywhere on the render thread:
//! render_diagnostics::bump_draw_call();
//! render_diagnostics::add_upload_bytes(bytes as u32);
//! ```
//!
//! All bump helpers are `#[inline]` and compile to a single atomic
//! `fetch_add`; they're safe to call from any thread but the
//! intended caller is the render thread.
//!
//! When no sink has been installed (typically in unit tests that
//! don't spin up the full stats pipeline), every bump is a no-op —
//! hot code doesn't need a guard.

use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, Ordering};

use shared::stats::DebugStats;

/// Global sink pointer.  Set once at render-thread startup; never
/// cleared except in unit tests via [`uninstall_for_tests`].  Using
/// a raw pointer in an `AtomicPtr` (rather than `OnceLock<Arc<_>>`)
/// keeps the read path branchless: a single atomic load + null check.
static SINK: AtomicPtr<DebugStats> = AtomicPtr::new(std::ptr::null_mut());

/// Install `stats` as the diagnostic sink.  Subsequent bump helpers
/// will drive that instance.  Calling `install` twice replaces the
/// prior sink and leaks the previous `Arc` — acceptable because the
/// sink is installed exactly once per host lifecycle.
pub fn install(stats: Arc<DebugStats>) {
    // Leak the Arc; the pointer lives for the rest of the process.
    // The alternative — juggling Arc via `Arc::into_raw` and
    // `Arc::from_raw` on every read — would force every bump to
    // temporarily reconstitute an Arc and increment its refcount.
    let raw = Arc::into_raw(stats) as *mut DebugStats;
    let prev = SINK.swap(raw, Ordering::Release);
    // Don't leak the previous `Arc` on reinstall; reclaim it.
    if !prev.is_null() {
        // SAFETY: prev was produced by an earlier `Arc::into_raw`
        // call; we hold the only strong reference to it once the
        // SINK slot has been swapped.
        unsafe { drop(Arc::from_raw(prev)) };
    }
}

/// Test-only helper: drop the sink so successive tests start with
/// a clean slate.
#[cfg(test)]
pub fn uninstall_for_tests() {
    let prev = SINK.swap(std::ptr::null_mut(), Ordering::Release);
    if !prev.is_null() {
        unsafe { drop(Arc::from_raw(prev)) };
    }
}

/// Crate-wide mutex that serialises every test which installs a
/// diagnostic sink AND every test whose production code path bumps
/// a metric via [`render_diagnostics`].  Without this, parallel
/// tests cross-contaminate: e.g. `paint.rs`'s shadow test bumps the
/// `shadow_filter_misses` counter on a sink installed by
/// `effect_cache.rs`'s metric test, throwing off delta assertions.
///
/// Lightweight to take — only a handful of tests need it — and it
/// forces deterministic serial execution for the short set.
#[cfg(test)]
pub static TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Convenience: acquire [`TEST_GUARD`] and return the guard,
/// recovering from poison so a previously-panicked test doesn't
/// cascade-poison later runs.
#[cfg(test)]
pub fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner())
}

#[inline]
fn with_sink(f: impl FnOnce(&DebugStats)) {
    let p = SINK.load(Ordering::Acquire);
    if !p.is_null() {
        // SAFETY: any non-null value in SINK was produced by
        // `Arc::into_raw` and is kept alive by the leaked refcount.
        let stats = unsafe { &*p };
        f(stats);
    }
}

// ---- Canvas2D / WebGL draw counters ---------------------------------

#[inline]
pub fn bump_draw_call() {
    with_sink(|s| {
        s.draw_calls.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn bump_state_change() {
    with_sink(|s| {
        s.state_changes.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn add_upload_bytes(bytes: u32) {
    with_sink(|s| {
        s.texture_upload_bytes.fetch_add(bytes, Ordering::Relaxed);
    });
}

// ---- Cache hit / miss bumps -----------------------------------------

#[inline]
pub fn hit_measure_cache() {
    with_sink(|s| {
        s.measure_text_hits.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn miss_measure_cache() {
    with_sink(|s| {
        s.measure_text_misses.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn hit_shape_cache() {
    with_sink(|s| {
        s.shape_cache_hits.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn miss_shape_cache() {
    with_sink(|s| {
        s.shape_cache_misses.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn hit_sk_image_wrapper() {
    with_sink(|s| {
        s.sk_image_wrapper_hits.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn miss_sk_image_wrapper() {
    with_sink(|s| {
        s.sk_image_wrapper_misses.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn bump_skia_context_reset() {
    with_sink(|s| {
        s.skia_context_resets.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn bump_canvas2d_snapshot_taken() {
    with_sink(|s| {
        s.canvas2d_snapshots_taken.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn bump_canvas2d_snapshot_fallback() {
    with_sink(|s| {
        s.canvas2d_snapshot_fallbacks
            .fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn bump_canvas2d_snapshot_upload() {
    with_sink(|s| {
        s.canvas2d_snapshot_uploads.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn bump_canvas2d_snapshot_forced_readback() {
    with_sink(|s| {
        s.canvas2d_snapshot_forced_readbacks
            .fetch_add(1, Ordering::Relaxed);
    });
}

/// Called from the `FrameOp::BeginFrame` handler.  Currently a
/// no-op because all per-frame diagnostics ride on the swap /
/// present path that sits later in the frame, but kept as a
/// named hook so future additions (frame-id, cpu-timer start)
/// can land without touching the render-thread code again
/// (P2-13).
#[inline]
pub fn frame_begin() {
    // Intentionally empty for now.  See doc comment.
}

#[inline]
pub fn hit_shadow_filter_cache() {
    with_sink(|s| {
        s.shadow_filter_hits.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn miss_shadow_filter_cache() {
    with_sink(|s| {
        s.shadow_filter_misses.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn hit_dash_effect_cache() {
    with_sink(|s| {
        s.dash_effect_hits.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn miss_dash_effect_cache() {
    with_sink(|s| {
        s.dash_effect_misses.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn hit_gradient_cache() {
    with_sink(|s| {
        s.gradient_hits.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn miss_gradient_cache() {
    with_sink(|s| {
        s.gradient_misses.fetch_add(1, Ordering::Relaxed);
    });
}

// ---- Text texture cache counters -----------------------------------

#[inline]
pub fn hit_text_cache() {
    with_sink(|s| {
        s.text_cache_hits.fetch_add(1, Ordering::Relaxed);
    });
}

#[inline]
pub fn miss_text_cache() {
    with_sink(|s| {
        s.text_cache_misses.fetch_add(1, Ordering::Relaxed);
    });
}

/// Publish the current resident bytes and entry count (gauges).
/// Called from the cache insert / evict / trim paths on the render
/// thread, *after* the cache lock is released so the snapshot we
/// store matches a consistent state.
#[inline]
pub fn set_text_cache_gauges(bytes: u32, entries: u32) {
    with_sink(|s| {
        s.text_cache_bytes.store(bytes, Ordering::Relaxed);
        s.text_cache_entries.store(entries, Ordering::Relaxed);
    });
}

// ---- v4 queue / collector / cache observability setters -------------

/// Record the current render command channel backlog.  Sampled by
/// the render thread at the top of each frame; the Java debug
/// overlay reads the latest value via `DebugStats::snapshot`.
#[inline]
pub fn set_render_queue_len(len: u32) {
    with_sink(|s| {
        s.render_queue_len.store(len, Ordering::Relaxed);
    });
}

/// Record the current live `SkImage` wrapper count.  `ImageStore`
/// calls this whenever the wrapper cache changes size.
#[inline]
pub fn set_sk_image_wrapper_count(count: u32) {
    with_sink(|s| {
        s.sk_image_wrappers.store(count, Ordering::Relaxed);
    });
}

/// Record the current deferred-upload queue depth (uploads waiting
/// for the next frame's budget).
#[inline]
pub fn set_deferred_uploads(count: u32) {
    with_sink(|s| {
        s.deferred_uploads.store(count, Ordering::Relaxed);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn bumps_are_noop_without_sink() {
        let _g = test_guard();
        uninstall_for_tests();
        bump_draw_call();
        add_upload_bytes(1024);
        hit_measure_cache();
    }

    #[test]
    fn bumps_route_to_installed_sink() {
        let _g = test_guard();
        uninstall_for_tests();
        let stats = Arc::new(DebugStats::default());
        install(stats.clone());
        bump_draw_call();
        bump_draw_call();
        add_upload_bytes(42);
        hit_measure_cache();
        miss_measure_cache();
        assert_eq!(stats.draw_calls.load(Ordering::Relaxed), 2);
        assert_eq!(stats.texture_upload_bytes.load(Ordering::Relaxed), 42);
        assert_eq!(stats.measure_text_hits.load(Ordering::Relaxed), 1);
        assert_eq!(stats.measure_text_misses.load(Ordering::Relaxed), 1);
        uninstall_for_tests();
    }

    #[test]
    fn reinstall_reclaims_previous_sink() {
        let _g = test_guard();
        uninstall_for_tests();
        let a = Arc::new(DebugStats::default());
        install(a.clone());
        let b = Arc::new(DebugStats::default());
        install(b.clone());
        bump_draw_call();
        assert_eq!(a.draw_calls.load(Ordering::Relaxed), 0);
        assert_eq!(b.draw_calls.load(Ordering::Relaxed), 1);
        uninstall_for_tests();
    }
}
