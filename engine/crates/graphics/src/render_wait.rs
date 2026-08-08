//! The render thread's single wait point.
//!
//! The loop has four wakeup sources and, on platforms with no external vsync,
//! one deadline. Putting them in one place makes idle quiescence checkable:
//! the wait is a function of the clock's deadline, so
//! `SoftwareFrameClock::deadline() == None` means the render thread sleeps until
//! a real event arrives, and nothing else in the loop can schedule a wakeup
//! behind its back.
//!
//! The deadline lives on the wait rather than in a timer channel on purpose.
//! `crossbeam_channel::at` would allocate a channel per armed frame — sixty
//! allocations a second on the render thread — while
//! [`Select::ready_deadline`](crossbeam_channel::Select::ready_deadline) takes
//! the deadline as an argument and costs nothing to re-arm.

use std::time::Instant;

use crossbeam_channel::{Receiver, Select};

/// Which source released the wait. A source that reports work may still find
/// none: `Select` readiness is advisory, so the loop treats an empty receiver as
/// a spurious wakeup and waits again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wake {
    SurfaceControl,
    Vsync,
    /// A demand source published while the engine-paced clock was stopped —
    /// `op_await_next_frame`'s arm route on platforms with no external vsync.
    FrameDemand,
    Command,
    /// The engine-paced clock's deadline came due.
    FrameDeadline,
}

pub(crate) struct RenderWait<'a> {
    select: Select<'a>,
    surface_control: usize,
    vsync: usize,
    frame_demand: usize,
    command: usize,
}

impl<'a> RenderWait<'a> {
    pub(crate) fn new<S, V, D, C>(
        surface_control: &'a Receiver<S>,
        vsync: &'a Receiver<V>,
        frame_demand: &'a Receiver<D>,
        command: &'a Receiver<C>,
    ) -> Self {
        let mut select = Select::new();
        Self {
            surface_control: select.recv(surface_control),
            vsync: select.recv(vsync),
            frame_demand: select.recv(frame_demand),
            command: select.recv(command),
            select,
        }
    }

    /// Block until a source has work or the frame deadline comes due. `None`
    /// waits indefinitely, which is what an idle engine-paced clock and every
    /// external-vsync platform ask for.
    pub(crate) fn next(&mut self, frame_deadline: Option<Instant>) -> Wake {
        let Some(deadline) = frame_deadline else {
            let index = self.select.ready();
            return self.wake_at(index);
        };

        // An overdue frame is served before the channels are polled. `Select`
        // returns any ready operation without consulting the deadline, so a
        // channel that is continuously ready would otherwise starve the frame
        // indefinitely. The reverse cannot happen: the frame branch drains the
        // command queue itself, and running a frame advances the pacing grid
        // past now.
        if Instant::now() >= deadline {
            return Wake::FrameDeadline;
        }

        match self.select.ready_deadline(deadline) {
            Ok(index) => self.wake_at(index),
            Err(_) => Wake::FrameDeadline,
        }
    }

    fn wake_at(&self, index: usize) -> Wake {
        if index == self.surface_control {
            Wake::SurfaceControl
        } else if index == self.vsync {
            Wake::Vsync
        } else if index == self.frame_demand {
            Wake::FrameDemand
        } else {
            debug_assert_eq!(index, self.command, "every registered source is named");
            Wake::Command
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RenderWait, Wake};
    use crate::frame_scheduler::SoftwareFrameClock;
    use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
    use std::time::{Duration, Instant};

    /// Long enough to cover fifteen frames at 60 Hz, so an engine-paced clock
    /// that woke on a fixed interval could not stay silent through it.
    const IDLE_WINDOW: Duration = Duration::from_millis(250);
    /// Bound on a wait that a correct implementation must release promptly.
    const PATIENCE: Duration = Duration::from_secs(2);

    struct Channels {
        surface_control: (Sender<()>, Receiver<()>),
        vsync: (Sender<f64>, Receiver<f64>),
        demand: (Sender<()>, Receiver<()>),
        command: (Sender<u32>, Receiver<u32>),
    }

    impl Channels {
        fn new() -> Self {
            Self {
                surface_control: unbounded(),
                vsync: unbounded(),
                demand: bounded(1),
                command: unbounded(),
            }
        }

        fn wait(&self) -> RenderWait<'_> {
            RenderWait::new(
                &self.surface_control.1,
                &self.vsync.1,
                &self.demand.1,
                &self.command.1,
            )
        }
    }

    #[test]
    fn an_idle_clock_never_wakes_the_render_thread() {
        let channels = Channels::new();
        let clock = SoftwareFrameClock::new(60);
        assert_eq!(clock.deadline(), None, "fixture: the clock is idle");

        let (woke_tx, woke_rx) = bounded(1);
        let releaser = channels.demand.0.clone();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let _ = woke_tx.send(channels.wait().next(clock.deadline()));
            });

            let woke = woke_rx.recv_timeout(IDLE_WINDOW);
            let released = releaser.try_send(());

            assert_eq!(
                woke.err(),
                Some(crossbeam_channel::RecvTimeoutError::Timeout),
                "an idle engine must schedule no wakeup at all; the wait returned \
                 {woke:?} within {IDLE_WINDOW:?}"
            );
            assert!(released.is_ok(), "the waiting thread is released");
        });
    }

    #[test]
    fn an_armed_clock_wakes_the_render_thread_at_its_deadline() {
        let channels = Channels::new();
        let mut clock = SoftwareFrameClock::new(60);
        clock.arm(Instant::now());

        assert_eq!(
            channels.wait().next(clock.deadline()),
            Wake::FrameDeadline,
            "the gate above must be able to pass for a reason other than a wait \
             that never returns"
        );
    }

    #[test]
    fn a_published_frame_demand_wakes_an_idle_clock() {
        let channels = Channels::new();
        let clock = SoftwareFrameClock::new(60);

        channels.demand.0.try_send(()).expect("nudge accepted");

        assert_eq!(
            channels.wait().next(clock.deadline()),
            Wake::FrameDemand,
            "stopping the clock is only safe because demand published while it is \
             stopped reaches the render thread"
        );
    }

    #[test]
    fn a_demand_nudge_that_arrives_during_the_wait_releases_it() {
        let channels = Channels::new();
        let clock = SoftwareFrameClock::new(60);
        let nudge = channels.demand.0.clone();

        std::thread::scope(|scope| {
            scope.spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                nudge.try_send(()).expect("nudge accepted");
            });
            assert_eq!(channels.wait().next(clock.deadline()), Wake::FrameDemand);
        });
    }

    #[test]
    fn each_wakeup_source_is_named_when_it_fires() {
        let channels = Channels::new();
        let idle = SoftwareFrameClock::new(60);

        channels.surface_control.0.send(()).expect("sent");
        assert_eq!(channels.wait().next(idle.deadline()), Wake::SurfaceControl);
        channels.surface_control.1.try_recv().expect("drained");

        channels.vsync.0.send(1.0).expect("sent");
        assert_eq!(channels.wait().next(idle.deadline()), Wake::Vsync);
        channels.vsync.1.try_recv().expect("drained");

        channels.command.0.send(7).expect("sent");
        assert_eq!(channels.wait().next(idle.deadline()), Wake::Command);
        channels.command.1.try_recv().expect("drained");
    }

    #[test]
    fn an_overdue_frame_is_served_before_a_ready_command_queue() {
        let channels = Channels::new();
        let mut clock = SoftwareFrameClock::new(60);
        clock.arm(Instant::now() - Duration::from_millis(50));
        channels.command.0.send(7).expect("sent");

        assert_eq!(
            channels.wait().next(clock.deadline()),
            Wake::FrameDeadline,
            "a permanently-ready channel must not starve an overdue frame: \
             readiness is checked without consulting the deadline"
        );
    }

    #[test]
    fn a_pending_command_wakes_the_thread_before_an_unexpired_deadline() {
        let channels = Channels::new();
        let mut clock = SoftwareFrameClock::new(1);
        clock.arm(Instant::now());
        clock.on_frame_ran(Instant::now());
        clock.arm(Instant::now());

        channels.command.0.send(7).expect("sent");

        let started = Instant::now();
        assert_eq!(channels.wait().next(clock.deadline()), Wake::Command);
        assert!(
            started.elapsed() < PATIENCE,
            "a command must not wait out a one-second frame slot"
        );
    }
}
