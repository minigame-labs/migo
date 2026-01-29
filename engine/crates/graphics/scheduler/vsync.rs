//! VSync and display timing utilities

use std::time::{Duration, Instant};

/// Display refresh rate detection and VSync timing
pub struct VSyncTimer {
    /// Target refresh interval
    refresh_interval: Duration,
    
    /// Last VSync timestamp
    last_vsync: Instant,
    
    /// Estimated next VSync
    next_vsync: Instant,
    
    /// Rolling average of actual frame times for calibration
    frame_times: [Duration; 8],
    frame_time_index: usize,
}

impl VSyncTimer {
    /// Create a new VSync timer for the given refresh rate
    pub fn new(refresh_rate_hz: u32) -> Self {
        let refresh_interval = Duration::from_secs_f64(1.0 / refresh_rate_hz as f64);
        let now = Instant::now();
        
        Self {
            refresh_interval,
            last_vsync: now,
            next_vsync: now + refresh_interval,
            frame_times: [refresh_interval; 8],
            frame_time_index: 0,
        }
    }
    
    /// Get the refresh interval
    pub fn refresh_interval(&self) -> Duration {
        self.refresh_interval
    }
    
    /// Update refresh rate (e.g., when display changes)
    pub fn set_refresh_rate(&mut self, refresh_rate_hz: u32) {
        self.refresh_interval = Duration::from_secs_f64(1.0 / refresh_rate_hz as f64);
    }
    
    /// Called after eglSwapBuffers to record actual VSync time
    pub fn record_vsync(&mut self) {
        let now = Instant::now();
        let actual_interval = now - self.last_vsync;
        
        // Update rolling average
        self.frame_times[self.frame_time_index] = actual_interval;
        self.frame_time_index = (self.frame_time_index + 1) % 8;
        
        self.last_vsync = now;
        self.next_vsync = now + self.refresh_interval;
    }
    
    /// Estimate time until next VSync
    pub fn time_until_vsync(&self) -> Duration {
        let now = Instant::now();
        if now >= self.next_vsync {
            Duration::ZERO
        } else {
            self.next_vsync - now
        }
    }
    
    /// Check if we have enough time to start another frame before next VSync
    pub fn can_start_frame(&self, estimated_frame_time: Duration) -> bool {
        self.time_until_vsync() > estimated_frame_time
    }
    
    /// Get the average actual frame time (for calibration)
    pub fn average_frame_time(&self) -> Duration {
        let total: Duration = self.frame_times.iter().sum();
        total / 8
    }
    
    /// Detect the actual display refresh rate from measurements
    pub fn detected_refresh_rate(&self) -> f64 {
        let avg = self.average_frame_time();
        if avg.as_secs_f64() > 0.0 {
            1.0 / avg.as_secs_f64()
        } else {
            60.0
        }
    }
}

impl Default for VSyncTimer {
    fn default() -> Self {
        Self::new(60)
    }
}

/// Frame deadline calculator for consistent frame pacing
pub struct FramePacer {
    /// Target frame time
    target_frame_time: Duration,
    
    /// Deadline for current frame
    deadline: Instant,
    
    /// Whether we're currently ahead or behind schedule
    timing_state: TimingState,
    
    /// Accumulated timing error for correction
    accumulated_error: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimingState {
    /// On schedule
    OnTime,
    /// Ahead of schedule (good)
    Ahead,
    /// Behind schedule (may need to drop frames)
    Behind,
}

impl FramePacer {
    pub fn new(target_fps: u32) -> Self {
        let target_frame_time = Duration::from_secs_f64(1.0 / target_fps as f64);
        let now = Instant::now();
        
        Self {
            target_frame_time,
            deadline: now + target_frame_time,
            timing_state: TimingState::OnTime,
            accumulated_error: Duration::ZERO,
        }
    }
    
    /// Start a new frame and get the deadline
    pub fn begin_frame(&mut self) -> Instant {
        let now = Instant::now();
        
        // Determine timing state
        if now < self.deadline {
            let ahead_by = self.deadline - now;
            if ahead_by > self.target_frame_time / 4 {
                self.timing_state = TimingState::Ahead;
            } else {
                self.timing_state = TimingState::OnTime;
            }
        } else {
            let behind_by = now - self.deadline;
            self.accumulated_error += behind_by;
            self.timing_state = TimingState::Behind;
        }
        
        // Calculate next deadline
        // If behind, try to catch up gradually (not all at once)
        if self.accumulated_error > Duration::ZERO {
            let correction = self.accumulated_error.min(self.target_frame_time / 10);
            self.accumulated_error -= correction;
            self.deadline = now + self.target_frame_time - correction;
        } else {
            self.deadline = now + self.target_frame_time;
        }
        
        self.deadline
    }
    
    /// Check timing state
    pub fn timing_state(&self) -> TimingState {
        self.timing_state
    }
    
    /// Check if we should drop a frame to catch up
    pub fn should_drop(&self) -> bool {
        self.accumulated_error > self.target_frame_time
    }
    
    /// Get time remaining until deadline
    pub fn time_remaining(&self) -> Option<Duration> {
        let now = Instant::now();
        if now < self.deadline {
            Some(self.deadline - now)
        } else {
            None
        }
    }
    
    /// Wait until the deadline (busy wait for precision)
    pub fn wait_for_deadline(&self) {
        let now = Instant::now();
        if now >= self.deadline {
            return;
        }
        
        let remaining = self.deadline - now;
        
        // Sleep for most of the time
        if remaining > Duration::from_millis(2) {
            std::thread::sleep(remaining - Duration::from_millis(1));
        }
        
        // Spin for the last bit for precision
        while Instant::now() < self.deadline {
            std::hint::spin_loop();
        }
    }
}
