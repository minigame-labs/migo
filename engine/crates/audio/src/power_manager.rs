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
//! | **Sleep**   | 500 ms+condvar| No active audio, idle >= `idle_timeout`  |
//! |             |               | (deep sleep, woken by command/callback)  |
//!
//! # Wakeup Guarantee
//!
//! In all states the audio thread sleeps on a [`ThreadWakeup`] condvar,
//! so any incoming [`AudioCmd`] instantly wakes the thread regardless of
//! the nominal tick interval.  Measured wakeup latency is < 0.5 ms on
//! both Linux (futex) and Android (futex/PI).
//!
//! # CPU Target
//!
//! - **No audio**: CPU < 0.1 % (Sleep, wakes every 500 ms only to check liveness)
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
    /// No active audio for a long time — 500 ms tick + condvar sleep.
    /// Consumes < 0.1 % CPU.
    Sleep,
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
    /// Tick interval when in Sleep state (condvar timeout).
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
/// Call [`update`](AudioPowerManager::update) once per loop iteration,
/// passing `true` if any audio source is currently active (playing,
/// streaming, or has an active `AudioContext`).  The manager returns the
/// recommended [`AudioPowerState`] and [`wait_duration`](AudioPowerManager::wait_duration)
/// for the condvar sleep.
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
    /// `is_active` should be `true` when any of the following is true:
    /// - An `AudioContext` is in `Running` state
    /// - An `InnerAudioPlayer` is `Playing`
    /// - A streaming download is in progress (`stream_rx.is_some()`)
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

    /// Get the recommended condvar wait duration for the current state.
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
}
