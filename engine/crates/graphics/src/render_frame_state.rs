use std::cell::Cell;
use std::time::Duration;

pub(crate) const DEFERRED_CLEANUP_INTERVAL: Duration = Duration::from_millis(250);
pub(crate) const DEFERRED_CLEANUP_UNUSED_AGE: Duration = Duration::from_millis(200);

/// Coalesces deferred Skia cleanup onto frames the renderer was already asked
/// to process. It never schedules a timer and advances directly to `now`, so a
/// stalled or paused renderer does not run catch-up sweeps.
pub(crate) struct DeferredCleanupCadence {
    last_run: Cell<Duration>,
}

impl DeferredCleanupCadence {
    pub(crate) fn new(now: Duration) -> Self {
        Self {
            last_run: Cell::new(now),
        }
    }

    #[inline]
    pub(crate) fn should_run(&self, now: Duration) -> bool {
        if now.saturating_sub(self.last_run.get()) < DEFERRED_CLEANUP_INTERVAL {
            return false;
        }
        self.last_run.set(now);
        true
    }

    #[inline]
    pub(crate) fn mark_forced(&self, now: Duration) {
        self.last_run.set(now);
    }
}

/// One render thread's diagnostic deltas and latest gauge samples. All fields
/// are plain integers so draw/cache hot paths do not touch shared atomics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FrameDiagnosticBatch {
    pub(crate) draw_calls: u32,
    pub(crate) state_changes: u32,
    pub(crate) texture_upload_bytes: u32,
    pub(crate) measure_text_hits: u32,
    pub(crate) measure_text_misses: u32,
    pub(crate) shape_cache_hits: u32,
    pub(crate) shape_cache_misses: u32,
    pub(crate) sk_image_wrapper_hits: u32,
    pub(crate) sk_image_wrapper_misses: u32,
    pub(crate) skia_context_resets: u32,
    pub(crate) canvas2d_snapshots_taken: u32,
    pub(crate) canvas2d_snapshot_fallbacks: u32,
    pub(crate) canvas2d_snapshot_uploads: u32,
    pub(crate) canvas2d_snapshot_forced_readbacks: u32,
    pub(crate) shadow_filter_hits: u32,
    pub(crate) shadow_filter_misses: u32,
    pub(crate) dash_effect_hits: u32,
    pub(crate) dash_effect_misses: u32,
    pub(crate) gradient_hits: u32,
    pub(crate) gradient_misses: u32,
    pub(crate) text_cache_hits: u32,
    pub(crate) text_cache_misses: u32,
    pub(crate) text_cache_bytes: Option<u32>,
    pub(crate) text_cache_entries: Option<u32>,
    pub(crate) render_queue_len: Option<u32>,
    pub(crate) sk_image_wrappers: Option<u32>,
    pub(crate) deferred_uploads: Option<u32>,
}

impl FrameDiagnosticBatch {
    const EMPTY: Self = Self {
        draw_calls: 0,
        state_changes: 0,
        texture_upload_bytes: 0,
        measure_text_hits: 0,
        measure_text_misses: 0,
        shape_cache_hits: 0,
        shape_cache_misses: 0,
        sk_image_wrapper_hits: 0,
        sk_image_wrapper_misses: 0,
        skia_context_resets: 0,
        canvas2d_snapshots_taken: 0,
        canvas2d_snapshot_fallbacks: 0,
        canvas2d_snapshot_uploads: 0,
        canvas2d_snapshot_forced_readbacks: 0,
        shadow_filter_hits: 0,
        shadow_filter_misses: 0,
        dash_effect_hits: 0,
        dash_effect_misses: 0,
        gradient_hits: 0,
        gradient_misses: 0,
        text_cache_hits: 0,
        text_cache_misses: 0,
        text_cache_bytes: None,
        text_cache_entries: None,
        render_queue_len: None,
        sk_image_wrappers: None,
        deferred_uploads: None,
    };
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FrameDiagnosticAccumulator {
    pending: FrameDiagnosticBatch,
}

macro_rules! counter_method {
    ($name:ident, $field:ident) => {
        #[inline]
        pub(crate) fn $name(&mut self) {
            self.pending.$field = self.pending.$field.saturating_add(1);
        }
    };
}

impl FrameDiagnosticAccumulator {
    pub(crate) const fn new() -> Self {
        Self {
            pending: FrameDiagnosticBatch::EMPTY,
        }
    }

    counter_method!(bump_draw_call, draw_calls);
    counter_method!(bump_state_change, state_changes);
    counter_method!(hit_measure_cache, measure_text_hits);
    counter_method!(miss_measure_cache, measure_text_misses);
    counter_method!(hit_shape_cache, shape_cache_hits);
    counter_method!(miss_shape_cache, shape_cache_misses);
    counter_method!(hit_sk_image_wrapper, sk_image_wrapper_hits);
    counter_method!(miss_sk_image_wrapper, sk_image_wrapper_misses);
    counter_method!(bump_skia_context_reset, skia_context_resets);
    counter_method!(bump_canvas2d_snapshot_taken, canvas2d_snapshots_taken);
    counter_method!(bump_canvas2d_snapshot_fallback, canvas2d_snapshot_fallbacks);
    counter_method!(bump_canvas2d_snapshot_upload, canvas2d_snapshot_uploads);
    counter_method!(
        bump_canvas2d_snapshot_forced_readback,
        canvas2d_snapshot_forced_readbacks
    );
    counter_method!(hit_shadow_filter_cache, shadow_filter_hits);
    counter_method!(miss_shadow_filter_cache, shadow_filter_misses);
    counter_method!(hit_dash_effect_cache, dash_effect_hits);
    counter_method!(miss_dash_effect_cache, dash_effect_misses);
    counter_method!(hit_gradient_cache, gradient_hits);
    counter_method!(miss_gradient_cache, gradient_misses);
    counter_method!(hit_text_cache, text_cache_hits);
    counter_method!(miss_text_cache, text_cache_misses);

    #[inline]
    pub(crate) fn add_upload_bytes(&mut self, bytes: u32) {
        self.pending.texture_upload_bytes = self.pending.texture_upload_bytes.saturating_add(bytes);
    }

    #[inline]
    pub(crate) fn set_text_cache_gauges(&mut self, bytes: u32, entries: u32) {
        self.pending.text_cache_bytes = Some(bytes);
        self.pending.text_cache_entries = Some(entries);
    }

    #[inline]
    pub(crate) fn set_render_queue_len(&mut self, len: u32) {
        self.pending.render_queue_len = Some(len);
    }

    #[inline]
    pub(crate) fn set_sk_image_wrapper_count(&mut self, count: u32) {
        self.pending.sk_image_wrappers = Some(count);
    }

    #[inline]
    pub(crate) fn set_deferred_uploads(&mut self, count: u32) {
        self.pending.deferred_uploads = Some(count);
    }

    #[inline]
    pub(crate) fn drain(&mut self) -> FrameDiagnosticBatch {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn cleanup_is_not_due_before_250_ms() {
        let cadence = DeferredCleanupCadence::new(Duration::from_millis(10));

        assert!(!cadence.should_run(Duration::from_millis(259)));
    }

    #[test]
    fn cleanup_is_due_at_250_ms() {
        let cadence = DeferredCleanupCadence::new(Duration::from_millis(10));

        assert!(cadence.should_run(Duration::from_millis(260)));
        assert!(!cadence.should_run(Duration::from_millis(260)));
    }

    #[test]
    fn cleanup_coalesces_missed_intervals_without_catch_up() {
        let cadence = DeferredCleanupCadence::new(Duration::ZERO);

        assert!(cadence.should_run(Duration::from_secs(5)));
        assert!(!cadence.should_run(Duration::from_secs(5)));
        assert!(!cadence.should_run(Duration::from_millis(5_249)));
        assert!(cadence.should_run(Duration::from_millis(5_250)));
    }

    #[test]
    fn forced_cleanup_restarts_the_deadline() {
        let cadence = DeferredCleanupCadence::new(Duration::ZERO);

        cadence.mark_forced(Duration::from_millis(240));
        assert!(!cadence.should_run(Duration::from_millis(489)));
        assert!(cadence.should_run(Duration::from_millis(490)));
    }

    #[test]
    fn regressed_timestamp_does_not_trigger_cleanup() {
        let cadence = DeferredCleanupCadence::new(Duration::from_secs(2));

        assert!(!cadence.should_run(Duration::from_secs(1)));
        assert!(!cadence.should_run(Duration::from_millis(2_249)));
        assert!(cadence.should_run(Duration::from_millis(2_250)));
    }

    #[test]
    fn diagnostic_drain_returns_deltas_and_resets_them() {
        let mut diagnostics = FrameDiagnosticAccumulator::default();
        diagnostics.bump_draw_call();
        diagnostics.bump_draw_call();
        diagnostics.hit_shadow_filter_cache();
        diagnostics.add_upload_bytes(42);

        let first = diagnostics.drain();
        assert_eq!(first.draw_calls, 2);
        assert_eq!(first.shadow_filter_hits, 1);
        assert_eq!(first.texture_upload_bytes, 42);

        let second = diagnostics.drain();
        assert_eq!(second.draw_calls, 0);
        assert_eq!(second.shadow_filter_hits, 0);
        assert_eq!(second.texture_upload_bytes, 0);
    }

    #[test]
    fn diagnostic_gauges_publish_latest_sample_only() {
        let mut diagnostics = FrameDiagnosticAccumulator::default();
        diagnostics.set_render_queue_len(7);
        diagnostics.set_render_queue_len(3);
        diagnostics.set_text_cache_gauges(8_192, 12);
        diagnostics.set_text_cache_gauges(4_096, 5);

        let batch = diagnostics.drain();
        assert_eq!(batch.render_queue_len, Some(3));
        assert_eq!(batch.text_cache_bytes, Some(4_096));
        assert_eq!(batch.text_cache_entries, Some(5));

        let empty = diagnostics.drain();
        assert_eq!(empty.render_queue_len, None);
        assert_eq!(empty.text_cache_bytes, None);
        assert_eq!(empty.text_cache_entries, None);
    }

    #[test]
    fn diagnostic_local_arithmetic_saturates() {
        let mut diagnostics = FrameDiagnosticAccumulator::default();
        diagnostics.add_upload_bytes(u32::MAX - 1);
        diagnostics.add_upload_bytes(10);
        for _ in 0..3 {
            diagnostics.bump_canvas2d_snapshot_taken();
        }

        let batch = diagnostics.drain();
        assert_eq!(batch.texture_upload_bytes, u32::MAX);
        assert_eq!(batch.canvas2d_snapshots_taken, 3);
    }

    #[test]
    fn render_thread_installs_and_flushes_session_diagnostics() {
        let source = include_str!("render_thread.rs");
        assert_eq!(
            source
                .matches("render_diagnostics::install(debug_stats.clone())")
                .count(),
            1,
            "one render thread must bind exactly one session-local sink"
        );
        assert_eq!(
            source.matches("render_diagnostics::flush_frame()").count(),
            1,
            "physical-frame tail must publish diagnostics exactly once"
        );
    }

    /// Presentation is coalesced by the physical-frame loop, so executing a
    /// packet's ops must never sweep every live context — that sweep is what
    /// made multiple packets in one display frame cost multiple all-context
    /// walks.
    ///
    /// **Scoped to the whole op executor rather than to the `Present` arm.** The
    /// earlier version split the source on `"FrameOp::Present => {"` and
    /// inspected the text up to the next arm. That pinned one arm out of five,
    /// so a sweep introduced in `Materialize` or `GlBatch` passed it; and it was
    /// coupled to the arm's brace style closely enough that turning the arm into
    /// an expression broke the extraction rather than the property. Both are the
    /// same mistake — a guard covering only the face it was written for — so the
    /// assertion now covers every arm and does not depend on how any of them is
    /// written.
    #[test]
    fn executing_a_frame_op_never_sweeps_every_context() {
        let source = include_str!("render_thread.rs");
        let executor = source
            .split_once("fn execute_frame_op(")
            .and_then(|(_, tail)| tail.split_once("\nfn "))
            .map(|(body, _)| body)
            .expect("execute_frame_op is where a packet's ops are executed");

        assert!(
            !executor.contains("perform_deferred_cleanup"),
            "executing one op performs the deferred-cleanup sweep, which belongs \
             to the physical-frame tail"
        );
        assert!(
            !executor.contains("contexts_2d_iter_mut"),
            "executing one op walks every live 2D context"
        );
    }

    #[test]
    fn periodic_and_pressure_cleanup_are_wired_without_a_timer() {
        let render = include_str!("render_thread.rs");
        let manager = include_str!("canvas/manager/mod.rs");

        assert!(render.contains("cleanup_cadence.should_run(start_time.elapsed())"));
        assert!(render.matches("cleanup_cadence.mark_forced").count() >= 2);
        assert!(render.contains("cm.perform_deferred_cleanup_all("));
        assert!(manager.contains("fn perform_deferred_cleanup_all("));
        assert!(!render.contains("tick(DEFERRED_CLEANUP_INTERVAL)"));
        assert!(!render.contains("sleep(DEFERRED_CLEANUP_INTERVAL)"));
    }
}
