//! Frame scheduler implementation

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, bounded};
use tracing::info;

use super::{FrameConfig, FrameStats, RenderMode, RollingStats};
use crate::command_buffer::CommandBuffer;

/// Message types for the frame scheduler
pub enum SchedulerMessage {
    /// A command buffer ready to be rendered
    Frame(CommandBuffer),
    
    /// Request to invalidate (redraw) - for on-demand mode
    Invalidate,
    
    /// Configuration change
    Configure(FrameConfig),
    
    /// Pause rendering
    Pause,
    
    /// Resume rendering
    Resume,
    
    /// Shutdown the scheduler
    Shutdown,
}

/// Response from the scheduler
pub enum SchedulerResponse {
    /// Ready for next frame
    Ready,
    
    /// Frame completed with stats
    FrameComplete(FrameStats),
    
    /// Scheduler has shut down
    Shutdown,
}

/// The frame scheduler coordinates timing between JS and render threads
///
/// It supports multiple rendering modes and provides frame pacing to
/// achieve smooth, consistent frame delivery.
pub struct FrameScheduler {
    /// Configuration
    config: FrameConfig,
    
    /// Frame statistics
    stats: RollingStats,
    
    /// Current frame number
    frame_number: AtomicU64,
    
    /// Whether invalidation is pending (for on-demand mode)
    invalidated: AtomicBool,
    
    /// Last frame timestamp
    last_frame_time: Instant,
    
    /// Target frame duration
    target_frame_duration: Duration,
}

impl FrameScheduler {
    /// Create a new frame scheduler with the given configuration
    pub fn new(config: FrameConfig) -> Self {
        let target_frame_duration = if config.target_fps > 0 {
            Duration::from_secs_f64(1.0 / config.target_fps as f64)
        } else {
            Duration::ZERO
        };
        
        Self {
            config,
            stats: RollingStats::new(120),
            frame_number: AtomicU64::new(0),
            invalidated: AtomicBool::new(true), // Start invalidated
            last_frame_time: Instant::now(),
            target_frame_duration,
        }
    }
    
    /// Update configuration
    pub fn configure(&mut self, config: FrameConfig) {
        info!("FrameScheduler: mode={:?}, fps={}, vsync={}", 
              config.mode, config.target_fps, config.vsync);
        
        self.target_frame_duration = if config.target_fps > 0 {
            Duration::from_secs_f64(1.0 / config.target_fps as f64)
        } else {
            Duration::ZERO
        };
        self.config = config;
    }
    
    /// Mark content as changed (for on-demand mode)
    pub fn invalidate(&self) {
        self.invalidated.store(true, Ordering::Release);
    }
    
    /// Check if a frame should be rendered
    pub fn should_render(&self) -> bool {
        match self.config.mode {
            RenderMode::Continuous => true,
            RenderMode::OnDemand => self.invalidated.load(Ordering::Acquire),
            RenderMode::Background => {
                // Reduced rate in background
                let elapsed = self.last_frame_time.elapsed();
                elapsed >= Duration::from_secs(1)
            }
            RenderMode::Paused => false,
        }
    }
    
    /// Wait for the appropriate time to start next frame
    pub fn wait_for_frame_start(&mut self) -> bool {
        if self.config.mode == RenderMode::Paused {
            return false;
        }
        
        if !self.should_render() {
            // In on-demand mode, wait for invalidation
            std::thread::sleep(Duration::from_millis(1));
            return false;
        }
        
        // Frame pacing: wait until target time
        if self.target_frame_duration > Duration::ZERO {
            let elapsed = self.last_frame_time.elapsed();
            if elapsed < self.target_frame_duration {
                let sleep_duration = self.target_frame_duration - elapsed;
                // Spin for last 1ms for precision
                if sleep_duration > Duration::from_millis(1) {
                    std::thread::sleep(sleep_duration - Duration::from_millis(1));
                }
                while self.last_frame_time.elapsed() < self.target_frame_duration {
                    std::hint::spin_loop();
                }
            }
        }
        
        true
    }
    
    /// Start a new frame
    pub fn begin_frame(&mut self) -> u64 {
        self.last_frame_time = Instant::now();
        self.invalidated.store(false, Ordering::Release);
        self.frame_number.fetch_add(1, Ordering::Relaxed)
    }
    
    /// End a frame and record statistics
    pub fn end_frame(&mut self, stats: FrameStats) {
        self.stats.record(&stats);
    }
    
    /// Get current statistics
    pub fn get_stats(&self) -> &RollingStats {
        &self.stats
    }
    
    /// Get current frame number
    pub fn frame_number(&self) -> u64 {
        self.frame_number.load(Ordering::Relaxed)
    }
    
    /// Get current rendering mode
    pub fn mode(&self) -> RenderMode {
        self.config.mode
    }
    
    /// Check if we should drop a frame (falling behind)
    pub fn should_drop_frame(&self) -> bool {
        if !self.config.allow_frame_drop {
            return false;
        }
        
        // If we're more than 2 frames behind, drop
        let elapsed = self.last_frame_time.elapsed();
        elapsed > self.target_frame_duration * 2
    }
}

/// Handle for sending frames to the scheduler
#[derive(Clone)]
pub struct FrameSubmitter {
    tx: Sender<SchedulerMessage>,
    invalidated: Arc<AtomicBool>,
}

impl FrameSubmitter {
    /// Submit a command buffer for rendering
    pub fn submit(&self, buffer: CommandBuffer) -> Result<(), &'static str> {
        self.tx.send(SchedulerMessage::Frame(buffer))
            .map_err(|_| "scheduler channel closed")
    }
    
    /// Request a redraw (for on-demand mode)
    pub fn invalidate(&self) {
        self.invalidated.store(true, Ordering::Release);
        let _ = self.tx.try_send(SchedulerMessage::Invalidate);
    }
    
    /// Configure the scheduler
    pub fn configure(&self, config: FrameConfig) -> Result<(), &'static str> {
        self.tx.send(SchedulerMessage::Configure(config))
            .map_err(|_| "scheduler channel closed")
    }
    
    /// Pause rendering
    pub fn pause(&self) -> Result<(), &'static str> {
        self.tx.send(SchedulerMessage::Pause)
            .map_err(|_| "scheduler channel closed")
    }
    
    /// Resume rendering
    pub fn resume(&self) -> Result<(), &'static str> {
        self.tx.send(SchedulerMessage::Resume)
            .map_err(|_| "scheduler channel closed")
    }
    
    /// Shutdown the scheduler
    pub fn shutdown(&self) -> Result<(), &'static str> {
        self.tx.send(SchedulerMessage::Shutdown)
            .map_err(|_| "scheduler channel closed")
    }
}

/// Create a frame scheduler pair (submitter, receiver)
pub fn create_scheduler_channel(capacity: usize) -> (FrameSubmitter, Receiver<SchedulerMessage>) {
    let (tx, rx) = bounded(capacity);
    let invalidated = Arc::new(AtomicBool::new(true));
    
    let submitter = FrameSubmitter {
        tx,
        invalidated,
    };
    
    (submitter, rx)
}
