//! Audio power management for battery optimization
//!
//! Manages audio thread power consumption by:
//! - Using callback-driven processing instead of polling
//! - Adjusting processing buffer size based on activity
//! - Suspending processing when no audio is playing

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Power state for audio subsystem
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioPowerState {
    /// Full processing, normal latency
    Active,
    /// Reduced processing, higher latency allowed
    LowPower,
    /// Minimal processing, app in background
    Background,
    /// No processing, audio suspended
    Suspended,
}

/// Configuration for audio power management
#[derive(Debug, Clone)]
pub struct AudioPowerConfig {
    /// Timeout before entering low power mode (ms)
    pub idle_timeout_ms: u64,
    /// Timeout before entering background mode (ms)
    pub background_timeout_ms: u64,
    /// Buffer size multiplier in low power mode
    pub low_power_buffer_multiplier: f32,
    /// Buffer size multiplier in background mode
    pub background_buffer_multiplier: f32,
    /// Whether to allow suspension
    pub allow_suspend: bool,
}

impl Default for AudioPowerConfig {
    fn default() -> Self {
        Self {
            idle_timeout_ms: 3000,      // 3 seconds
            background_timeout_ms: 10000, // 10 seconds
            low_power_buffer_multiplier: 2.0,
            background_buffer_multiplier: 4.0,
            allow_suspend: true,
        }
    }
}

/// Shared state for cross-thread access
pub struct SharedAudioPowerState {
    /// Current power state
    state: AtomicU32,
    /// Whether audio is actively playing
    audio_active: AtomicBool,
    /// Last activity timestamp (microseconds since start)
    last_activity_us: AtomicU64,
    /// Whether app is in foreground
    app_foreground: AtomicBool,
}

impl SharedAudioPowerState {
    pub fn new() -> Self {
        Self {
            state: AtomicU32::new(AudioPowerState::Active as u32),
            audio_active: AtomicBool::new(false),
            last_activity_us: AtomicU64::new(0),
            app_foreground: AtomicBool::new(true),
        }
    }
    
    /// Get current power state
    pub fn state(&self) -> AudioPowerState {
        match self.state.load(Ordering::Acquire) {
            0 => AudioPowerState::Active,
            1 => AudioPowerState::LowPower,
            2 => AudioPowerState::Background,
            _ => AudioPowerState::Suspended,
        }
    }
    
    /// Set power state
    pub fn set_state(&self, state: AudioPowerState) {
        self.state.store(state as u32, Ordering::Release);
    }
    
    /// Mark audio as active
    pub fn set_audio_active(&self, active: bool) {
        self.audio_active.store(active, Ordering::Release);
    }
    
    /// Check if audio is active
    pub fn is_audio_active(&self) -> bool {
        self.audio_active.load(Ordering::Acquire)
    }
    
    /// Record activity
    pub fn record_activity(&self, timestamp_us: u64) {
        self.last_activity_us.store(timestamp_us, Ordering::Release);
    }
    
    /// Get last activity timestamp
    pub fn last_activity_us(&self) -> u64 {
        self.last_activity_us.load(Ordering::Acquire)
    }
    
    /// Set app foreground state
    pub fn set_app_foreground(&self, foreground: bool) {
        self.app_foreground.store(foreground, Ordering::Release);
    }
    
    /// Check if app is in foreground
    pub fn is_app_foreground(&self) -> bool {
        self.app_foreground.load(Ordering::Acquire)
    }
}

impl Default for SharedAudioPowerState {
    fn default() -> Self {
        Self::new()
    }
}

/// Audio power manager
pub struct AudioPowerManager {
    config: AudioPowerConfig,
    shared: Arc<SharedAudioPowerState>,
    start_time: Instant,
    /// Base buffer size (samples)
    base_buffer_size: usize,
    /// Current buffer size
    current_buffer_size: usize,
}

impl AudioPowerManager {
    /// Create a new power manager
    pub fn new(config: AudioPowerConfig, base_buffer_size: usize) -> Self {
        Self {
            config,
            shared: Arc::new(SharedAudioPowerState::new()),
            start_time: Instant::now(),
            base_buffer_size,
            current_buffer_size: base_buffer_size,
        }
    }
    
    /// Get shared state for other threads
    pub fn shared(&self) -> Arc<SharedAudioPowerState> {
        self.shared.clone()
    }
    
    /// Update power state based on current conditions
    pub fn update(&mut self) -> AudioPowerState {
        let now_us = self.start_time.elapsed().as_micros() as u64;
        let last_activity_us = self.shared.last_activity_us();
        let idle_time_ms = (now_us - last_activity_us) / 1000;
        
        let is_active = self.shared.is_audio_active();
        let is_foreground = self.shared.is_app_foreground();
        
        let new_state = if !is_foreground {
            // App in background
            if self.config.allow_suspend && !is_active {
                AudioPowerState::Suspended
            } else {
                AudioPowerState::Background
            }
        } else if is_active {
            // Audio is playing
            AudioPowerState::Active
        } else if idle_time_ms > self.config.background_timeout_ms {
            // Long idle, can suspend
            if self.config.allow_suspend {
                AudioPowerState::Suspended
            } else {
                AudioPowerState::LowPower
            }
        } else if idle_time_ms > self.config.idle_timeout_ms {
            // Short idle, low power mode
            AudioPowerState::LowPower
        } else {
            AudioPowerState::Active
        };
        
        // Update buffer size based on state
        self.current_buffer_size = match new_state {
            AudioPowerState::Active => self.base_buffer_size,
            AudioPowerState::LowPower => {
                (self.base_buffer_size as f32 * self.config.low_power_buffer_multiplier) as usize
            }
            AudioPowerState::Background => {
                (self.base_buffer_size as f32 * self.config.background_buffer_multiplier) as usize
            }
            AudioPowerState::Suspended => self.base_buffer_size,
        };
        
        self.shared.set_state(new_state);
        new_state
    }
    
    /// Record audio activity
    pub fn record_activity(&self) {
        let now_us = self.start_time.elapsed().as_micros() as u64;
        self.shared.record_activity(now_us);
    }
    
    /// Called when audio starts playing
    pub fn on_audio_start(&self) {
        self.shared.set_audio_active(true);
        self.record_activity();
    }
    
    /// Called when audio stops playing
    pub fn on_audio_stop(&self) {
        self.shared.set_audio_active(false);
    }
    
    /// Called when app goes to foreground
    pub fn on_app_foreground(&self) {
        self.shared.set_app_foreground(true);
        self.record_activity();
    }
    
    /// Called when app goes to background
    pub fn on_app_background(&self) {
        self.shared.set_app_foreground(false);
    }
    
    /// Get current buffer size
    pub fn buffer_size(&self) -> usize {
        self.current_buffer_size
    }
    
    /// Get recommended wait duration
    pub fn wait_duration(&self) -> Duration {
        match self.shared.state() {
            AudioPowerState::Active => Duration::from_millis(5),
            AudioPowerState::LowPower => Duration::from_millis(20),
            AudioPowerState::Background => Duration::from_millis(100),
            AudioPowerState::Suspended => Duration::from_millis(500),
        }
    }
    
    /// Check if processing should be skipped
    pub fn should_skip_processing(&self) -> bool {
        self.shared.state() == AudioPowerState::Suspended
    }
}

/// Callback-driven audio sync mechanism
pub struct AudioCallbackSync {
    /// Whether the callback needs data
    needs_data: AtomicBool,
    /// Minimum buffer fill level
    min_fill: AtomicU32,
    /// Current buffer fill level
    current_fill: AtomicU32,
}

impl AudioCallbackSync {
    pub fn new() -> Self {
        Self {
            needs_data: AtomicBool::new(false),
            min_fill: AtomicU32::new(4096),
            current_fill: AtomicU32::new(0),
        }
    }
    
    /// Called by audio callback when it needs more data
    pub fn request_data(&self) {
        self.needs_data.store(true, Ordering::Release);
    }
    
    /// Check and clear the data request
    pub fn check_and_clear(&self) -> bool {
        self.needs_data.swap(false, Ordering::AcqRel)
    }
    
    /// Update current fill level
    pub fn set_fill_level(&self, level: u32) {
        self.current_fill.store(level, Ordering::Release);
    }
    
    /// Get current fill level
    pub fn fill_level(&self) -> u32 {
        self.current_fill.load(Ordering::Acquire)
    }
    
    /// Check if buffer needs filling
    pub fn needs_fill(&self) -> bool {
        self.current_fill.load(Ordering::Acquire) < self.min_fill.load(Ordering::Acquire)
    }
    
    /// Set minimum fill level
    pub fn set_min_fill(&self, level: u32) {
        self.min_fill.store(level, Ordering::Release);
    }
}

impl Default for AudioCallbackSync {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for AudioCallbackSync {
    fn clone(&self) -> Self {
        Self {
            needs_data: AtomicBool::new(self.needs_data.load(Ordering::Relaxed)),
            min_fill: AtomicU32::new(self.min_fill.load(Ordering::Relaxed)),
            current_fill: AtomicU32::new(self.current_fill.load(Ordering::Relaxed)),
        }
    }
}
