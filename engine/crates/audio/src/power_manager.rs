//! Audio thread 3-level power management.
//!
//! # Power States
//!
//! | State       | Tick Interval | Entry Condition                          |
//! |-------------|---------------|------------------------------------------|
//! | **Active**  | 5 ms          | Any context running / player playing /   |
//! |             |               | streaming download in progress            |
//! | **LowPower**| 50 ms         | No active audio, idle < `idle_timeout`   |
//! |             |               | (recently stopped, may resume soon)      |
//! | **Sleep**   | event-driven  | No active audio, idle >= `idle_timeout`  |
//! |             |               | (output paused, woken by command)        |
//!
//! # Wakeup Guarantee
//!
//! In all states the audio thread sleeps on a [`ThreadWakeup`] condvar, so
//! incoming [`AudioCmd`] values wake the thread regardless of the nominal
//! tick interval.
//!
//! # CPU Target
//!
//! - **No audio**: event-driven wait after the hardware stream is paused
//! - **Audio playing**: same as before (Active, 5 ms tick)
//! - **Audio just stopped**: LowPower for a short window, then Sleep

use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Power state enum
// ---------------------------------------------------------------------------

/// 3-level power state for the audio thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioPowerState {
    /// Active audio processing — 5 ms tick for low-latency mixing.
    Active,
    /// No active audio for a short time — 50 ms tick.
    /// Keeps the thread warm so a quick Play command has minimal ramp-up.
    LowPower,
    /// No active audio for a long time — hardware stream paused and an
    /// event-driven condvar wait.
    Sleep,
}

/// A hardware stream transition requested by [`AudioStreamGate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AudioStreamAction {
    Pause,
    Resume,
}

/// How the audio loop should yield after completing one iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AudioWaitMode {
    /// Commands may remain beyond the bounded drain; start another iteration.
    Continue,
    /// The hardware stream is stopped and only an explicit event is needed.
    Indefinite,
    /// Mixing, an idle deadline, or a failed pause still needs a timed retry.
    Timed(Duration),
}

pub(crate) fn audio_wait_mode(
    commands_may_remain: bool,
    app_paused: bool,
    management_state: AudioPowerState,
    stream_running: bool,
    state_wait: Duration,
    pause_retry: Duration,
) -> AudioWaitMode {
    if commands_may_remain {
        AudioWaitMode::Continue
    } else if app_paused {
        if stream_running {
            AudioWaitMode::Timed(pause_retry)
        } else {
            AudioWaitMode::Indefinite
        }
    } else if management_state == AudioPowerState::Sleep && !stream_running {
        AudioWaitMode::Indefinite
    } else {
        AudioWaitMode::Timed(state_wait)
    }
}

/// Tracks the last successfully applied hardware stream state.
///
/// The audio thread owns this value, so it does not need atomics or locking.
/// Callers must commit an action only after the corresponding CPAL operation
/// succeeds; otherwise `next_action` deliberately returns it again.
pub(crate) struct AudioStreamGate {
    running: bool,
}

impl AudioStreamGate {
    /// `AudioOutput::new` starts CPAL before returning.
    pub(crate) fn new_running() -> Self {
        Self { running: true }
    }

    /// Return the stream transition required by the current lifecycle and
    /// power state, without changing the tracked state.
    pub(crate) fn next_action(
        &self,
        app_paused: bool,
        power_state: AudioPowerState,
    ) -> Option<AudioStreamAction> {
        let desired_running = if app_paused || power_state == AudioPowerState::Sleep {
            Some(false)
        } else if power_state == AudioPowerState::Active {
            Some(true)
        } else {
            // Keep the current state during the warm window. In particular,
            // foregrounding an idle app must not restart a silent stream.
            None
        };

        match desired_running {
            Some(true) if !self.running => Some(AudioStreamAction::Resume),
            Some(false) if self.running => Some(AudioStreamAction::Pause),
            _ => None,
        }
    }

    /// Record a successfully applied stream transition.
    pub(crate) fn commit(&mut self, action: AudioStreamAction) {
        self.running = action == AudioStreamAction::Resume;
    }

    /// Record that a newly created or recovered output starts in play state.
    pub(crate) fn mark_running(&mut self) {
        self.running = true;
    }

    /// Record that an output error stopped the current stream. Recovery can
    /// then be deferred until audible work exists.
    pub(crate) fn mark_stopped(&mut self) {
        self.running = false;
    }

    pub(crate) fn is_running(&self) -> bool {
        self.running
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Tunable thresholds for power state transitions.
#[derive(Debug, Clone)]
pub struct AudioPowerConfig {
    /// How long to stay in LowPower after the last active audio stops
    /// before transitioning to Sleep.
    /// Default: 3 seconds.
    pub idle_timeout: Duration,

    /// Tick interval when in Active state.
    pub active_tick: Duration,
    /// Tick interval when in LowPower state.
    pub low_power_tick: Duration,
    /// Retry interval when Sleep cannot enter an indefinite wait, for example
    /// because pausing the hardware stream failed.
    pub sleep_tick: Duration,
}

impl Default for AudioPowerConfig {
    fn default() -> Self {
        Self {
            idle_timeout: Duration::from_secs(3),
            active_tick: Duration::from_millis(5),
            low_power_tick: Duration::from_millis(50),
            sleep_tick: Duration::from_millis(500),
        }
    }
}

// ---------------------------------------------------------------------------
// Power manager (lives on the audio thread, no Arc/Atomic needed)
// ---------------------------------------------------------------------------

/// Manages the audio thread's power state transitions.
///
/// Call [`update`](AudioPowerManager::update) once per loop iteration, passing
/// `true` for the activity class that instance manages. The audio thread uses
/// one instance for management work (including streaming) and a second for
/// audible output. The manager returns the recommended [`AudioPowerState`] and
/// timed-wait fallback.
pub struct AudioPowerManager {
    config: AudioPowerConfig,
    state: AudioPowerState,
    /// Timestamp of the most recent transition from Active → non-Active.
    last_active_time: Instant,
}

impl AudioPowerManager {
    /// Create a new power manager starting in Active state.
    pub fn new(config: AudioPowerConfig) -> Self {
        Self {
            config,
            state: AudioPowerState::Active,
            last_active_time: Instant::now(),
        }
    }

    /// Update the power state based on current activity.
    ///
    /// The caller defines what constitutes activity for this instance.
    ///
    /// Returns the new power state.
    pub fn update(&mut self, is_active: bool) -> AudioPowerState {
        if is_active {
            // Don't update last_active_time while active — Instant::now()
            // is a syscall on some platforms. We only need the timestamp
            // when transitioning from Active → inactive.
            self.state = AudioPowerState::Active;
        } else {
            if self.state == AudioPowerState::Active {
                // Transitioning from Active to inactive — record the time once.
                self.last_active_time = Instant::now();
            }
            let idle = self.last_active_time.elapsed();
            self.state = if idle < self.config.idle_timeout {
                AudioPowerState::LowPower
            } else {
                AudioPowerState::Sleep
            };
        }
        self.state
    }

    /// Get the timed condvar interval for the current state. Sleep normally
    /// waits indefinitely; its duration is a failure/retry fallback.
    #[inline]
    pub fn wait_duration(&self) -> Duration {
        match self.state {
            AudioPowerState::Active => self.config.active_tick,
            AudioPowerState::LowPower => self.config.low_power_tick,
            AudioPowerState::Sleep => self.config.sleep_tick,
        }
    }

    /// Current power state.
    #[inline]
    pub fn state(&self) -> AudioPowerState {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_is_active() {
        let mgr = AudioPowerManager::new(AudioPowerConfig::default());
        assert_eq!(mgr.state(), AudioPowerState::Active);
        assert_eq!(mgr.wait_duration(), Duration::from_millis(5));
    }

    #[test]
    fn stays_active_while_playing() {
        let mut mgr = AudioPowerManager::new(AudioPowerConfig::default());
        for _ in 0..10 {
            let s = mgr.update(true);
            assert_eq!(s, AudioPowerState::Active);
        }
    }

    #[test]
    fn transitions_to_low_power_on_idle() {
        let config = AudioPowerConfig {
            idle_timeout: Duration::from_millis(50),
            ..Default::default()
        };
        let mut mgr = AudioPowerManager::new(config);

        // First update with no activity — records timestamp, enters LowPower
        mgr.update(false);
        assert_eq!(mgr.state(), AudioPowerState::LowPower);
        assert_eq!(mgr.wait_duration(), Duration::from_millis(50));
    }

    #[test]
    fn transitions_to_sleep_after_idle_timeout() {
        let config = AudioPowerConfig {
            idle_timeout: Duration::from_millis(20),
            ..Default::default()
        };
        let mut mgr = AudioPowerManager::new(config);

        mgr.update(false); // Active -> LowPower
        std::thread::sleep(Duration::from_millis(30));
        mgr.update(false); // idle > 20ms -> Sleep
        assert_eq!(mgr.state(), AudioPowerState::Sleep);
        assert_eq!(mgr.wait_duration(), Duration::from_millis(500));
    }

    #[test]
    fn returns_to_active_on_activity() {
        let config = AudioPowerConfig {
            idle_timeout: Duration::from_millis(10),
            ..Default::default()
        };
        let mut mgr = AudioPowerManager::new(config);

        mgr.update(false);
        std::thread::sleep(Duration::from_millis(20));
        mgr.update(false);
        assert_eq!(mgr.state(), AudioPowerState::Sleep);

        // Activity resumes
        mgr.update(true);
        assert_eq!(mgr.state(), AudioPowerState::Active);
        assert_eq!(mgr.wait_duration(), Duration::from_millis(5));
    }

    #[test]
    fn low_power_stays_until_timeout() {
        let config = AudioPowerConfig {
            idle_timeout: Duration::from_millis(200),
            ..Default::default()
        };
        let mut mgr = AudioPowerManager::new(config);

        mgr.update(false); // -> LowPower
        // 50ms is well under the 200ms timeout
        std::thread::sleep(Duration::from_millis(50));
        mgr.update(false);
        assert_eq!(mgr.state(), AudioPowerState::LowPower);
    }

    #[test]
    fn config_defaults_are_sane() {
        let config = AudioPowerConfig::default();
        assert_eq!(config.idle_timeout, Duration::from_secs(3));
        assert_eq!(config.active_tick, Duration::from_millis(5));
        assert_eq!(config.low_power_tick, Duration::from_millis(50));
        assert_eq!(config.sleep_tick, Duration::from_millis(500));
    }

    #[test]
    fn rapid_active_inactive_toggle() {
        let config = AudioPowerConfig {
            idle_timeout: Duration::from_millis(100),
            ..Default::default()
        };
        let mut mgr = AudioPowerManager::new(config);

        // Rapid toggling should not reach Sleep
        for _ in 0..20 {
            mgr.update(false);
            mgr.update(true);
        }
        assert_eq!(mgr.state(), AudioPowerState::Active);
    }

    #[test]
    fn output_can_sleep_while_streaming_keeps_management_active() {
        let config = AudioPowerConfig {
            idle_timeout: Duration::ZERO,
            ..Default::default()
        };
        let mut management = AudioPowerManager::new(config.clone());
        let mut output = AudioPowerManager::new(config);
        let gate = AudioStreamGate::new_running();

        assert_eq!(management.update(true), AudioPowerState::Active);
        let output_state = output.update(false);
        assert_eq!(output_state, AudioPowerState::Sleep);
        assert_eq!(
            gate.next_action(false, output_state),
            Some(AudioStreamAction::Pause)
        );
    }

    #[test]
    fn stream_gate_keeps_warm_then_pauses_once_in_sleep() {
        let mut gate = AudioStreamGate::new_running();

        assert_eq!(gate.next_action(false, AudioPowerState::LowPower), None);
        assert!(gate.is_running());

        assert_eq!(
            gate.next_action(false, AudioPowerState::Sleep),
            Some(AudioStreamAction::Pause)
        );
        // A failed CPAL call must leave the action pending.
        assert!(gate.is_running());
        assert_eq!(
            gate.next_action(false, AudioPowerState::Sleep),
            Some(AudioStreamAction::Pause)
        );

        gate.commit(AudioStreamAction::Pause);
        assert!(!gate.is_running());
        assert_eq!(gate.next_action(false, AudioPowerState::Sleep), None);
    }

    #[test]
    fn foregrounding_an_idle_session_does_not_restart_silence() {
        let mut gate = AudioStreamGate::new_running();

        assert_eq!(
            gate.next_action(true, AudioPowerState::Active),
            Some(AudioStreamAction::Pause)
        );
        gate.commit(AudioStreamAction::Pause);

        assert_eq!(gate.next_action(false, AudioPowerState::LowPower), None);
        assert!(!gate.is_running());
    }

    #[test]
    fn active_audio_resumes_a_stream_paused_in_background() {
        let mut gate = AudioStreamGate::new_running();
        gate.commit(AudioStreamAction::Pause);

        assert_eq!(
            gate.next_action(false, AudioPowerState::Active),
            Some(AudioStreamAction::Resume)
        );
        gate.commit(AudioStreamAction::Resume);
        assert!(gate.is_running());
        assert_eq!(gate.next_action(false, AudioPowerState::Active), None);
    }

    #[test]
    fn recovered_stream_is_reconciled_with_sleep_state() {
        let mut gate = AudioStreamGate::new_running();
        gate.commit(AudioStreamAction::Pause);
        gate.mark_running();

        assert_eq!(
            gate.next_action(false, AudioPowerState::Sleep),
            Some(AudioStreamAction::Pause)
        );
    }

    #[test]
    fn wait_mode_covers_command_backlog_lifecycle_and_power_states() {
        let active_tick = Duration::from_millis(5);
        let sleep_retry = Duration::from_millis(500);
        let cases = [
            (
                true,
                true,
                AudioPowerState::Sleep,
                false,
                AudioWaitMode::Continue,
            ),
            (
                true,
                false,
                AudioPowerState::Sleep,
                false,
                AudioWaitMode::Continue,
            ),
            (
                false,
                true,
                AudioPowerState::Sleep,
                true,
                AudioWaitMode::Timed(sleep_retry),
            ),
            (
                false,
                true,
                AudioPowerState::Active,
                false,
                AudioWaitMode::Indefinite,
            ),
            (
                false,
                false,
                AudioPowerState::Sleep,
                false,
                AudioWaitMode::Indefinite,
            ),
            (
                false,
                false,
                AudioPowerState::Sleep,
                true,
                AudioWaitMode::Timed(active_tick),
            ),
            (
                false,
                false,
                AudioPowerState::Active,
                false,
                AudioWaitMode::Timed(active_tick),
            ),
        ];

        for (commands, paused, power, running, expected) in cases {
            assert_eq!(
                audio_wait_mode(commands, paused, power, running, active_tick, sleep_retry,),
                expected
            );
        }

        let low_power_tick = Duration::from_millis(37);
        assert_eq!(
            audio_wait_mode(
                false,
                false,
                AudioPowerState::LowPower,
                false,
                low_power_tick,
                sleep_retry,
            ),
            AudioWaitMode::Timed(low_power_tick)
        );
    }
}
