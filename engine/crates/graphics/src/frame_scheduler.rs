use std::time::{Duration, Instant};

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

/// The engine-paced frame clock, used on platforms that deliver no vsync
/// callbacks: Linux, Windows, HarmonyOS, and any C host that did not install
/// `on_request_frame`.
///
/// It answers one question — when, if ever, should the render thread wake for a
/// frame — so the loop's wait has a single source. `None` means never: the clock
/// is idle and only a real event (a command, a surface change, a published RAF
/// demand) can wake the thread. That is what makes idle quiescence a property of
/// this type rather than of the loop that reads it.
///
/// Two `Option<Instant>`s, because they answer different questions and have
/// different lifetimes. `armed_at` is the deadline of the scheduled frame, so
/// `None` is precisely "idle" and there is no way to spell "armed for no
/// particular time". `earliest_next` is the pacing grid, kept *across* an idle
/// period so demand republished inside a frame's own slot waits for it instead of
/// starting a second frame — without that, a rAF loop would drive the clock as
/// fast as JS could ask.
pub struct SoftwareFrameClock {
    interval: Duration,
    armed_at: Option<Instant>,
    earliest_next: Option<Instant>,
}

impl SoftwareFrameClock {
    /// An idle clock at `fps` (clamped to the range the frame rate op accepts).
    pub fn new(fps: u32) -> Self {
        Self {
            interval: Self::interval_for(fps),
            armed_at: None,
            earliest_next: None,
        }
    }

    fn interval_for(fps: u32) -> Duration {
        Duration::from_secs(1) / fps.clamp(1, 120)
    }

    /// Change the frame interval. Deliberately does not arm: a frame rate is a
    /// pace, not demand. An already-armed frame keeps its deadline and the new
    /// interval takes effect from the slot after it.
    pub fn set_fps(&mut self, fps: u32) {
        self.interval = Self::interval_for(fps);
    }

    /// Demand exists — schedule one frame. Idempotent, so every demand source
    /// may call it without checking.
    pub fn arm(&mut self, now: Instant) {
        if self.armed_at.is_some() {
            return;
        }
        let at = match self.earliest_next {
            Some(slot) if slot > now => slot,
            // The clock was idle through this slot (or has never run a frame),
            // so the grid is stale: run immediately and re-phase from this
            // wakeup. Keeping the stale phase would let the frame after this one
            // land less than an interval later.
            _ => now,
        };
        self.earliest_next = Some(at);
        self.armed_at = Some(at);
    }

    /// Retire the pending wakeup without disturbing the pacing grid, so a
    /// pause-resume shorter than one frame interval cannot produce two frames in
    /// that interval.
    pub fn stop(&mut self) {
        self.armed_at = None;
    }

    /// When the render thread should wake for a frame; `None` while idle.
    pub fn deadline(&self) -> Option<Instant> {
        self.armed_at
    }

    /// The armed frame ran. Advances the grid to its first slot strictly after
    /// `ran_at` and leaves the clock idle, so the next frame happens only if a
    /// demand source arms it again.
    ///
    /// The slot is computed rather than iterated: the grid is
    /// `earliest_next + k * interval`, so dropping the partial interval `ran_at`
    /// sits in and adding a whole one lands on the next slot whether the frame
    /// was two milliseconds or two minutes late. A frame that overran its slot
    /// therefore owes exactly one frame, not one per slot it missed, and a frame
    /// that ran on time keeps the grid phase — lateness never becomes drift.
    pub fn on_frame_ran(&mut self, ran_at: Instant) {
        self.armed_at = None;
        let slot = self.earliest_next.unwrap_or(ran_at);
        let into_slot =
            ran_at.saturating_duration_since(slot).as_nanos() % self.interval.as_nanos();
        self.earliest_next = Some(ran_at + self.interval - Duration::from_nanos(into_slot as u64));
    }
}

/// Whether the engine-paced software clock should arm one more frame.
///
/// The sibling of [`should_arm_one_shot`] for platforms that deliver no vsync
/// callbacks (Linux, Windows, HarmonyOS, and any C host that did not install
/// `on_request_frame`). Both routes arm on demand only; this one deliberately
/// omits `can_present`, because the two arms buy different things. Asking a
/// compositor for a frame callback with no live surface is meaningless, whereas
/// an engine-paced frame with no surface still opens the per-frame upload budget
/// and drains completed uploads — which a host that loads content before handing
/// over its window depends on. Its cost is bounded by demand either way: no
/// demand, no wakeup.
#[inline]
pub fn should_arm_engine_paced(paused: bool, demand_remains: bool) -> bool {
    !paused && demand_remains
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
        // no vsync source -> never arm through this route; that platform is
        // paced by `should_arm_engine_paced` instead
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

    #[test]
    fn engine_paced_arm_needs_demand_and_a_running_clock() {
        use super::should_arm_engine_paced;
        assert!(should_arm_engine_paced(false, true), "demand => arm");
        assert!(
            !should_arm_engine_paced(false, false),
            "no demand => no wakeup (idle quiescence)"
        );
        assert!(
            !should_arm_engine_paced(true, true),
            "paused => no wakeup even with retained demand"
        );
    }
}

/// Deterministic tests for the engine-paced clock. Every instant is computed,
/// never slept on, so the pacing grid is asserted exactly.
#[cfg(test)]
mod software_frame_clock_tests {
    use super::SoftwareFrameClock;
    use std::time::{Duration, Instant};

    /// The interval the clock must derive from `fps`, spelled independently so a
    /// change to its arithmetic is caught rather than mirrored.
    fn interval(fps: u32) -> Duration {
        Duration::from_secs(1) / fps
    }

    #[test]
    fn an_idle_clock_schedules_no_wakeup() {
        let clock = SoftwareFrameClock::new(60);
        assert_eq!(
            clock.deadline(),
            None,
            "a clock with no demand must not wake the render thread at all"
        );
    }

    #[test]
    fn a_frame_that_ran_leaves_the_clock_idle_until_demand_re_arms_it() {
        let t0 = Instant::now();
        let mut clock = SoftwareFrameClock::new(60);

        clock.arm(t0);
        clock.on_frame_ran(t0);

        assert_eq!(
            clock.deadline(),
            None,
            "the clock does not free-run: a frame is followed by a wakeup only if \
             demand remains"
        );
    }

    #[test]
    fn the_first_armed_frame_runs_without_waiting_for_a_slot() {
        let t0 = Instant::now();
        let mut clock = SoftwareFrameClock::new(60);

        clock.arm(t0);

        assert_eq!(
            clock.deadline(),
            Some(t0),
            "demand arriving at a stopped clock must not pay a frame of latency"
        );
    }

    #[test]
    fn re_arming_inside_the_current_slot_cannot_raise_the_frame_rate() {
        let t0 = Instant::now();
        let mut clock = SoftwareFrameClock::new(60);

        clock.arm(t0);
        clock.on_frame_ran(t0);
        clock.arm(t0 + Duration::from_millis(2));

        assert_eq!(
            clock.deadline(),
            Some(t0 + interval(60)),
            "demand republished 2ms after a frame waits for that frame's slot, so \
             a rAF loop cannot spin the clock"
        );
    }

    #[test]
    fn arming_an_already_armed_clock_leaves_its_deadline_alone() {
        let t0 = Instant::now();
        let mut clock = SoftwareFrameClock::new(60);

        clock.arm(t0);
        clock.on_frame_ran(t0);
        clock.arm(t0 + Duration::from_millis(2));
        clock.arm(t0 + Duration::from_millis(9));

        assert_eq!(clock.deadline(), Some(t0 + interval(60)));
    }

    #[test]
    fn pacing_does_not_drift_when_every_frame_runs_late() {
        let t0 = Instant::now();
        let mut clock = SoftwareFrameClock::new(60);
        let lateness = Duration::from_millis(2);
        clock.arm(t0);

        for slots_elapsed in 1..=10u32 {
            let slot = clock.deadline().expect("armed while demand remains");
            let ran_at = slot + lateness;
            clock.on_frame_ran(ran_at);
            clock.arm(ran_at);
            assert_eq!(
                clock.deadline(),
                Some(t0 + interval(60) * slots_elapsed),
                "slot {slots_elapsed} stays on the grid: lateness must not accumulate"
            );
        }
    }

    #[test]
    fn a_frame_that_overran_its_slot_resumes_on_the_next_slot_instead_of_bursting() {
        let t0 = Instant::now();
        let mut clock = SoftwareFrameClock::new(60);
        clock.arm(t0);

        let overran_by = interval(60) * 10 + Duration::from_millis(3);
        clock.on_frame_ran(t0 + overran_by);
        clock.arm(t0 + overran_by);

        assert_eq!(
            clock.deadline(),
            Some(t0 + interval(60) * 11),
            "ten missed slots owe one frame, not ten: the clock skips to the first \
             slot after the frame that overran"
        );
    }

    #[test]
    fn a_clock_armed_after_a_long_idle_re_phases_from_the_wakeup() {
        let t0 = Instant::now();
        let mut clock = SoftwareFrameClock::new(60);
        clock.arm(t0);
        clock.on_frame_ran(t0);

        let woke = t0 + Duration::from_secs(5);
        clock.arm(woke);
        assert_eq!(
            clock.deadline(),
            Some(woke),
            "a frame demanded after idle runs immediately"
        );

        clock.on_frame_ran(woke);
        clock.arm(woke);
        assert_eq!(
            clock.deadline(),
            Some(woke + interval(60)),
            "and the grid re-phases from that frame, so the one after it is a whole \
             interval away rather than landing on the stale grid"
        );
    }

    #[test]
    fn stopping_cancels_the_armed_wakeup() {
        let t0 = Instant::now();
        let mut clock = SoftwareFrameClock::new(60);

        clock.arm(t0);
        clock.stop();

        assert_eq!(
            clock.deadline(),
            None,
            "pause must retire the pending wakeup, not merely ignore it"
        );
    }

    #[test]
    fn a_lowered_frame_rate_widens_the_next_slot() {
        let t0 = Instant::now();
        let mut clock = SoftwareFrameClock::new(60);

        clock.set_fps(30);
        clock.arm(t0);
        clock.on_frame_ran(t0);
        clock.arm(t0);

        assert_eq!(clock.deadline(), Some(t0 + interval(30)));
    }

    #[test]
    fn the_frame_rate_is_clamped_to_the_supported_range() {
        let t0 = Instant::now();
        let mut clock = SoftwareFrameClock::new(0);
        clock.arm(t0);
        clock.on_frame_ran(t0);
        clock.arm(t0);
        assert_eq!(
            clock.deadline(),
            Some(t0 + interval(1)),
            "0 fps clamps to 1"
        );

        let mut clock = SoftwareFrameClock::new(1000);
        clock.arm(t0);
        clock.on_frame_ran(t0);
        clock.arm(t0);
        assert_eq!(
            clock.deadline(),
            Some(t0 + interval(120)),
            "1000 fps clamps to 120"
        );
    }
}
