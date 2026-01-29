//! # Frame Scheduler
//!
//! Intelligent frame scheduling with support for:
//! - VSync-driven rendering (60/90/120 Hz)
//! - On-demand rendering (for UI, low power)
//! - Adaptive frame rate
//!
//! ## Rendering Modes
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                     Continuous Mode (Games)                          │
//! │                                                                      │
//! │   VSync ─┬─────────┬─────────┬─────────┬─────────→                  │
//! │          │         │         │         │                             │
//! │   RAF    ▼         ▼         ▼         ▼                             │
//! │   Frame: [  1  ]   [  2  ]   [  3  ]   [  4  ]                       │
//! │                                                                      │
//! └─────────────────────────────────────────────────────────────────────┘
//!
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                    On-Demand Mode (UI Apps)                          │
//! │                                                                      │
//! │   Event ──●───────────────●──────●────────────────●──→              │
//! │           │               │      │                │                  │
//! │   Frame:  [1]             [2]    [3]              [4]                │
//! │                                                                      │
//! │   No events = No rendering = Zero GPU power                          │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```

mod frame_scheduler;
mod vsync;
mod adaptive;

pub use frame_scheduler::*;
pub use vsync::*;
pub use adaptive::*;

/// Rendering mode selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderMode {
    /// Continuous rendering at fixed frame rate (games)
    #[default]
    Continuous,
    
    /// Only render when content changes (UI apps, power saving)
    OnDemand,
    
    /// Render at reduced rate when app is in background
    Background,
    
    /// Pause all rendering (app minimized)
    Paused,
}

/// Frame timing configuration
#[derive(Debug, Clone)]
pub struct FrameConfig {
    /// Target frame rate (0 = unlimited)
    pub target_fps: u32,
    
    /// Whether to wait for VSync
    pub vsync: bool,
    
    /// Rendering mode
    pub mode: RenderMode,
    
    /// Maximum frames to buffer ahead (for pipelining)
    pub max_frames_in_flight: u32,
    
    /// Whether to drop frames if behind
    pub allow_frame_drop: bool,
}

impl Default for FrameConfig {
    fn default() -> Self {
        Self {
            target_fps: 60,
            vsync: true,
            mode: RenderMode::Continuous,
            max_frames_in_flight: 2,
            allow_frame_drop: true,
        }
    }
}

impl FrameConfig {
    /// Configuration optimized for games
    pub fn game() -> Self {
        Self {
            target_fps: 60,
            vsync: true,
            mode: RenderMode::Continuous,
            max_frames_in_flight: 2,
            allow_frame_drop: true,
        }
    }
    
    /// Configuration optimized for UI apps (power saving)
    pub fn ui() -> Self {
        Self {
            target_fps: 60,
            vsync: true,
            mode: RenderMode::OnDemand,
            max_frames_in_flight: 1,
            allow_frame_drop: false,
        }
    }
    
    /// Configuration for background/minimized apps
    pub fn background() -> Self {
        Self {
            target_fps: 1,
            vsync: false,
            mode: RenderMode::Background,
            max_frames_in_flight: 1,
            allow_frame_drop: true,
        }
    }
}

/// Frame statistics for profiling and debugging
#[derive(Debug, Clone, Default)]
pub struct FrameStats {
    /// Frame number
    pub frame_number: u64,
    
    /// Time spent encoding commands (JS side)
    pub encode_time_us: u64,
    
    /// Time spent executing commands (Render side)
    pub execute_time_us: u64,
    
    /// Time waiting for GPU
    pub gpu_wait_time_us: u64,
    
    /// Time waiting for VSync
    pub vsync_wait_time_us: u64,
    
    /// Total frame time
    pub total_time_us: u64,
    
    /// Number of commands in this frame
    pub command_count: u32,
    
    /// Number of draw calls
    pub draw_call_count: u32,
    
    /// Whether this frame was dropped
    pub dropped: bool,
}

impl FrameStats {
    /// Calculate FPS from total time
    pub fn fps(&self) -> f32 {
        if self.total_time_us == 0 {
            0.0
        } else {
            1_000_000.0 / self.total_time_us as f32
        }
    }
    
    /// Check if frame met its deadline
    pub fn met_deadline(&self, target_fps: u32) -> bool {
        if target_fps == 0 {
            return true;
        }
        let target_time_us = 1_000_000 / target_fps as u64;
        self.total_time_us <= target_time_us
    }
}

/// Rolling statistics over multiple frames
#[derive(Debug, Clone)]
pub struct RollingStats {
    /// Window size for rolling average
    window_size: usize,
    /// Recent frame times (microseconds)
    frame_times: Vec<u64>,
    /// Current index in circular buffer
    index: usize,
    /// Whether buffer is full
    full: bool,
    /// Total frames processed
    total_frames: u64,
    /// Dropped frame count
    dropped_frames: u64,
}

impl RollingStats {
    pub fn new(window_size: usize) -> Self {
        Self {
            window_size,
            frame_times: vec![0; window_size],
            index: 0,
            full: false,
            total_frames: 0,
            dropped_frames: 0,
        }
    }
    
    pub fn record(&mut self, stats: &FrameStats) {
        self.frame_times[self.index] = stats.total_time_us;
        self.index = (self.index + 1) % self.window_size;
        if self.index == 0 {
            self.full = true;
        }
        self.total_frames += 1;
        if stats.dropped {
            self.dropped_frames += 1;
        }
    }
    
    /// Get average FPS over the window
    pub fn average_fps(&self) -> f32 {
        let count = if self.full { self.window_size } else { self.index };
        if count == 0 {
            return 0.0;
        }
        
        let total: u64 = self.frame_times[..count].iter().sum();
        let avg_time_us = total / count as u64;
        
        if avg_time_us == 0 {
            0.0
        } else {
            1_000_000.0 / avg_time_us as f32
        }
    }
    
    /// Get frame drop rate (0.0 - 1.0)
    pub fn drop_rate(&self) -> f32 {
        if self.total_frames == 0 {
            0.0
        } else {
            self.dropped_frames as f32 / self.total_frames as f32
        }
    }
    
    /// Get 99th percentile frame time
    pub fn p99_frame_time_us(&self) -> u64 {
        let count = if self.full { self.window_size } else { self.index };
        if count == 0 {
            return 0;
        }
        
        let mut sorted: Vec<u64> = self.frame_times[..count].to_vec();
        sorted.sort_unstable();
        
        let p99_idx = (count * 99 / 100).max(1) - 1;
        sorted[p99_idx]
    }
}

impl Default for RollingStats {
    fn default() -> Self {
        Self::new(60) // Default to 1 second window at 60 FPS
    }
}
