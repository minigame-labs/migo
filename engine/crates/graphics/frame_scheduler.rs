pub struct FrameDecision {
    pub should_render: bool,
    pub raf_time_ms: f64,
}

pub struct FrameScheduler {
    preferred_fps: u32,
    time_origin_ms: Option<f64>,
    next_deadline_ms: Option<f64>,
}

impl FrameScheduler {
    pub fn new(preferred_fps: u32) -> Self {
        Self {
            preferred_fps: preferred_fps.clamp(1, 120),
            time_origin_ms: None,
            next_deadline_ms: None,
        }
    }

    pub fn on_vsync(&mut self, vsync_ts_ms: f64) -> FrameDecision {
        let origin = *self.time_origin_ms.get_or_insert(vsync_ts_ms);
        let raf_time_ms = vsync_ts_ms - origin;
        let frame_interval_ms = 1000.0 / self.preferred_fps as f64;
        let deadline = self.next_deadline_ms.get_or_insert(0.0);
        let should_render = raf_time_ms + 0.25 >= *deadline;

        if should_render {
            let mut next_deadline_ms = *deadline + frame_interval_ms;

            if next_deadline_ms <= raf_time_ms {
                let skipped_slots =
                    ((raf_time_ms - next_deadline_ms) / frame_interval_ms).floor() + 1.0;
                next_deadline_ms += skipped_slots * frame_interval_ms;
            }

            while next_deadline_ms <= raf_time_ms {
                next_deadline_ms += frame_interval_ms;
            }

            *deadline = next_deadline_ms;
        }

        FrameDecision {
            should_render,
            raf_time_ms,
        }
    }

    pub fn set_preferred_fps(&mut self, preferred_fps: u32) {
        self.preferred_fps = preferred_fps.clamp(1, 120);
    }
}

/// Demand for the on-demand vsync clock: any of an actual RAF waiter, dirty
/// content awaiting present, outstanding upload work (deferred uploads or an
/// unsignalled fence), or a pending EGL context-recovery retry. Zero demand =>
/// the clock stops.
#[inline]
pub fn raf_demand_remains(
    waiter_pending: bool,
    dirty: bool,
    upload_work: bool,
    recovery_pending: bool,
) -> bool {
    waiter_pending || dirty || upload_work || recovery_pending
}

/// Whether the render thread should request exactly one more display frame.
/// Gated so an idle, paused, or surfaceless engine never arms (no spin, no JNI
/// flood), and `already_armed` suppresses a redundant request while one is in
/// flight (Java's own latch is the authoritative dedup).
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn should_arm_one_shot(
    has_vsync: bool,
    arm_available: bool,
    paused: bool,
    can_present: bool,
    demand_remains: bool,
    already_armed: bool,
) -> bool {
    has_vsync && arm_available && !paused && can_present && demand_remains && !already_armed
}

/// True when uploads still need a frame to make progress: completed uploads
/// whose fence has not yet signalled (`pending`), budget-rejected uploads
/// awaiting a fresh frame budget (`deferred`), or in-flight jobs submitted to
/// the upload thread but not yet drained (`in_flight`). Any of these is an
/// on-demand vsync source, so an async upload completing while the frame clock
/// is idle still nudges exactly one frame to poll the fence / register it.
#[inline]
pub fn outstanding_upload_work(pending: usize, deferred: usize, in_flight: u32) -> bool {
    pending > 0 || deferred > 0 || in_flight > 0
}

#[cfg(test)]
pub(crate) fn assert_hits_60fps_on_90hz_without_jittering_to_45fps() {
    let mut scheduler = FrameScheduler::new(60);
    let mut presented = 0;
    for ts_ms in [
        0.0, 11.111, 22.222, 33.333, 44.444, 55.555, 66.666, 77.777, 88.888,
    ] {
        if scheduler.on_vsync(ts_ms).should_render {
            presented += 1;
        }
    }
    assert_eq!(presented, 6);
}

#[cfg(test)]
pub(crate) fn assert_session_relative_time_starts_from_first_vsync() {
    let mut scheduler = FrameScheduler::new(60);
    let first = scheduler.on_vsync(5.0);
    let second = scheduler.on_vsync(21.666);
    assert_eq!(first.raf_time_ms, 0.0);
    assert!(second.raf_time_ms > 0.0);
}

#[cfg(test)]
pub(crate) fn assert_resynchronizes_after_long_stall() {
    let mut scheduler = FrameScheduler::new(60);

    assert!(scheduler.on_vsync(0.0).should_render);
    assert!(scheduler.on_vsync(100.0).should_render);
    assert!(!scheduler.on_vsync(111.111).should_render);
    assert!(scheduler.on_vsync(116.667).should_render);
}

#[cfg(test)]
pub(crate) fn assert_does_not_present_when_surface_is_not_ready() {
    use crate::SurfaceSystem;

    let mut scheduler = FrameScheduler::new(60);
    let surface = SurfaceSystem::new();

    let decision = scheduler.on_vsync(0.0);
    assert!(decision.should_render);
    assert!(!surface.can_present());
}

#[cfg(test)]
mod tests {
    use super::{
        assert_does_not_present_when_surface_is_not_ready,
        assert_hits_60fps_on_90hz_without_jittering_to_45fps,
        assert_resynchronizes_after_long_stall,
        assert_session_relative_time_starts_from_first_vsync,
    };

    #[test]
    fn hits_60fps_on_90hz_without_jittering_to_45fps() {
        assert_hits_60fps_on_90hz_without_jittering_to_45fps();
    }

    #[test]
    fn session_relative_time_starts_from_first_vsync() {
        assert_session_relative_time_starts_from_first_vsync();
    }

    #[test]
    fn resynchronizes_after_long_stall() {
        assert_resynchronizes_after_long_stall();
    }

    #[test]
    fn does_not_present_when_surface_is_not_ready() {
        assert_does_not_present_when_surface_is_not_ready();
    }

    #[test]
    fn demand_remains_true_if_any_source_active() {
        use super::raf_demand_remains;
        assert!(!raf_demand_remains(false, false, false, false));
        assert!(raf_demand_remains(true, false, false, false)); // RAF waiter
        assert!(raf_demand_remains(false, true, false, false)); // dirty
        assert!(raf_demand_remains(false, false, true, false)); // upload work
        assert!(raf_demand_remains(false, false, false, true)); // context recovery
    }

    #[test]
    fn arm_only_when_vsync_source_demand_and_presentable() {
        use super::should_arm_one_shot;
        // canonical arm: has_vsync, arm available, not paused, can present, demand, not already armed
        assert!(should_arm_one_shot(true, true, false, true, true, false));
        // no demand -> no arm (idle stops the clock)
        assert!(!should_arm_one_shot(true, true, false, true, false, false));
        // paused -> no arm (no spin while paused)
        assert!(!should_arm_one_shot(true, true, true, true, true, false));
        // no live surface -> no arm (no spin without surface)
        assert!(!should_arm_one_shot(true, true, false, false, true, false));
        // already armed -> suppress redundant JNI
        assert!(!should_arm_one_shot(true, true, false, true, true, true));
        // no vsync source (desktop ticker) -> never arm
        assert!(!should_arm_one_shot(false, true, false, true, true, false));
        // no arm closure -> never arm
        assert!(!should_arm_one_shot(true, false, false, true, true, false));
    }

    #[test]
    fn outstanding_upload_work_counts_pending_deferred_and_in_flight() {
        use super::outstanding_upload_work;
        assert!(
            !outstanding_upload_work(0, 0, 0),
            "no upload work => clock may stop"
        );
        assert!(
            outstanding_upload_work(1, 0, 0),
            "fence-pending completed upload is work"
        );
        assert!(
            outstanding_upload_work(0, 1, 0),
            "budget-deferred retry is work"
        );
        assert!(
            outstanding_upload_work(0, 0, 1),
            "in-flight (submitted, undrained) upload is work"
        );
    }
}
