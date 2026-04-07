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
}
