//! Adaptive frame rate scheduling
//!
//! Automatically adjusts frame rate based on:
//! - Scene complexity
//! - Battery level
//! - Thermal state
//! - App state (foreground/background)

use std::time::{Duration, Instant};

/// Frame complexity metrics
#[derive(Debug, Clone, Default)]
pub struct FrameComplexity {
    /// Number of draw calls
    pub draw_calls: u32,
    /// Number of state changes
    pub state_changes: u32,
    /// Number of texture switches
    pub texture_switches: u32,
    /// Total triangles rendered
    pub triangles: u32,
    /// Time spent in CPU (microseconds)
    pub cpu_time_us: u64,
    /// Time spent waiting for GPU (microseconds)
    pub gpu_time_us: u64,
}

impl FrameComplexity {
    /// Calculate complexity score (0.0 - 1.0)
    pub fn score(&self) -> f32 {
        // Weighted combination of metrics
        let draw_score = (self.draw_calls as f32 / 100.0).min(1.0);
        let tri_score = (self.triangles as f32 / 10000.0).min(1.0);
        let time_score = (self.cpu_time_us as f32 / 10000.0).min(1.0);
        
        (draw_score * 0.3 + tri_score * 0.4 + time_score * 0.3).min(1.0)
    }
}

/// Power state of the device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    /// Plugged in, full performance
    Charging,
    /// Battery, normal usage
    Battery,
    /// Low battery, power saving
    LowBattery,
    /// Critical battery, minimal rendering
    Critical,
}

/// Thermal state of the device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalState {
    /// Normal temperature
    Normal,
    /// Slightly warm, reduce performance
    Fair,
    /// Hot, significant reduction
    Serious,
    /// Overheating, minimal rendering
    Critical,
}

/// Configuration for adaptive frame rate
#[derive(Debug, Clone)]
pub struct AdaptiveConfig {
    /// Maximum FPS (when conditions are optimal)
    pub max_fps: u32,
    /// Minimum FPS (never go below this)
    pub min_fps: u32,
    /// Target frame time margin (1.0 = no margin, 1.2 = 20% margin)
    pub frame_time_margin: f32,
    /// Smoothing factor for FPS changes (0-1, higher = smoother)
    pub smoothing: f32,
    /// Whether to consider battery state
    pub battery_aware: bool,
    /// Whether to consider thermal state
    pub thermal_aware: bool,
}

impl Default for AdaptiveConfig {
    fn default() -> Self {
        Self {
            max_fps: 60,
            min_fps: 15,
            frame_time_margin: 1.1,
            smoothing: 0.9,
            battery_aware: true,
            thermal_aware: true,
        }
    }
}

/// Adaptive frame rate controller
pub struct AdaptiveFrameRate {
    config: AdaptiveConfig,
    /// Current target FPS
    current_fps: f32,
    /// Rolling average of frame times
    avg_frame_time_us: f32,
    /// Rolling average of complexity
    avg_complexity: f32,
    /// Current power state
    power_state: PowerState,
    /// Current thermal state
    thermal_state: ThermalState,
    /// Last update time
    last_update: Instant,
    /// Frame count since last update
    frame_count: u32,
}

impl AdaptiveFrameRate {
    /// Create with default config
    pub fn new() -> Self {
        Self::with_config(AdaptiveConfig::default())
    }
    
    /// Create with custom config
    pub fn with_config(config: AdaptiveConfig) -> Self {
        Self {
            current_fps: config.max_fps as f32,
            config,
            avg_frame_time_us: 16666.0, // Assume 60 FPS initially
            avg_complexity: 0.5,
            power_state: PowerState::Battery,
            thermal_state: ThermalState::Normal,
            last_update: Instant::now(),
            frame_count: 0,
        }
    }
    
    /// Update with new frame metrics
    pub fn update(&mut self, complexity: &FrameComplexity) {
        self.frame_count += 1;
        
        // Update rolling averages
        let alpha = 1.0 - self.config.smoothing;
        self.avg_frame_time_us = self.avg_frame_time_us * self.config.smoothing 
            + complexity.cpu_time_us as f32 * alpha;
        self.avg_complexity = self.avg_complexity * self.config.smoothing 
            + complexity.score() * alpha;
        
        // Only adjust FPS periodically (every 10 frames or 200ms)
        let elapsed = self.last_update.elapsed();
        if self.frame_count < 10 && elapsed < Duration::from_millis(200) {
            return;
        }
        
        self.frame_count = 0;
        self.last_update = Instant::now();
        
        // Calculate ideal FPS based on frame time
        let target_frame_time_us = 1_000_000.0 / self.config.max_fps as f32;
        let ideal_fps = if self.avg_frame_time_us > 0.0 {
            (1_000_000.0 / (self.avg_frame_time_us * self.config.frame_time_margin))
                .min(self.config.max_fps as f32)
        } else {
            self.config.max_fps as f32
        };
        
        // Apply power state modifier
        let power_modifier = match self.power_state {
            PowerState::Charging => 1.0,
            PowerState::Battery => 1.0,
            PowerState::LowBattery => 0.75,
            PowerState::Critical => 0.5,
        };
        
        // Apply thermal state modifier
        let thermal_modifier = match self.thermal_state {
            ThermalState::Normal => 1.0,
            ThermalState::Fair => 0.9,
            ThermalState::Serious => 0.7,
            ThermalState::Critical => 0.5,
        };
        
        // Calculate final target FPS
        let mut target_fps = ideal_fps;
        
        if self.config.battery_aware {
            target_fps *= power_modifier;
        }
        
        if self.config.thermal_aware {
            target_fps *= thermal_modifier;
        }
        
        // Clamp to configured range
        target_fps = target_fps.clamp(self.config.min_fps as f32, self.config.max_fps as f32);
        
        // Smooth transition
        self.current_fps = self.current_fps * self.config.smoothing 
            + target_fps * (1.0 - self.config.smoothing);
    }
    
    /// Get current recommended FPS
    pub fn recommended_fps(&self) -> u32 {
        self.current_fps.round() as u32
    }
    
    /// Get recommended frame duration
    pub fn recommended_frame_duration(&self) -> Duration {
        Duration::from_secs_f64(1.0 / self.current_fps as f64)
    }
    
    /// Set power state
    pub fn set_power_state(&mut self, state: PowerState) {
        self.power_state = state;
    }
    
    /// Set thermal state
    pub fn set_thermal_state(&mut self, state: ThermalState) {
        self.thermal_state = state;
    }
    
    /// Force a specific FPS (disables adaptation temporarily)
    pub fn force_fps(&mut self, fps: u32) {
        self.current_fps = fps as f32;
    }
    
    /// Get current state info for debugging
    pub fn debug_info(&self) -> String {
        format!(
            "FPS: {:.1}, FrameTime: {:.1}us, Complexity: {:.2}, Power: {:?}, Thermal: {:?}",
            self.current_fps,
            self.avg_frame_time_us,
            self.avg_complexity,
            self.power_state,
            self.thermal_state
        )
    }
}

impl Default for AdaptiveFrameRate {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_complexity_score() {
        let low = FrameComplexity {
            draw_calls: 10,
            triangles: 1000,
            cpu_time_us: 2000,
            ..Default::default()
        };
        
        let high = FrameComplexity {
            draw_calls: 200,
            triangles: 50000,
            cpu_time_us: 15000,
            ..Default::default()
        };
        
        assert!(low.score() < high.score());
        assert!(low.score() < 0.5);
        assert!(high.score() > 0.5);
    }
    
    #[test]
    fn test_adaptive_fps() {
        let mut adaptive = AdaptiveFrameRate::new();
        
        // Simulate heavy frames
        for _ in 0..20 {
            let complexity = FrameComplexity {
                cpu_time_us: 20000, // 20ms per frame
                ..Default::default()
            };
            adaptive.update(&complexity);
        }
        
        // Should reduce FPS
        assert!(adaptive.recommended_fps() < 60);
    }
}
