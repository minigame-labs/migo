//! # Dirty Region Tracking
//!
//! Tracks changed regions of the screen to enable partial redraws,
//! reducing GPU workload and power consumption.
//!
//! ## How It Works
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │                         Frame N                                         │
//! │   ┌───────────────────────────────┐                                    │
//! │   │                               │                                    │
//! │   │      ┌─────┐                  │   Only the marked dirty regions    │
//! │   │      │DIRTY│                  │   need to be redrawn.              │
//! │   │      └─────┘                  │                                    │
//! │   │                    ┌───┐      │   Unchanged areas are preserved    │
//! │   │                    │ D │      │   from the previous frame.         │
//! │   │                    └───┘      │                                    │
//! │   └───────────────────────────────┘                                    │
//! └─────────────────────────────────────────────────────────────────────────┘
//! ```

mod rect;
mod tracker;

pub use rect::*;
pub use tracker::*;

/// Statistics for dirty region optimization
#[derive(Debug, Clone, Default)]
pub struct DirtyStats {
    /// Frames with partial updates
    pub partial_updates: u64,
    /// Frames with full redraws
    pub full_redraws: u64,
    /// Total dirty area (pixels)
    pub total_dirty_pixels: u64,
    /// Total screen area (pixels)  
    pub total_screen_pixels: u64,
}

impl DirtyStats {
    /// Calculate average savings (0.0 - 1.0)
    pub fn savings_ratio(&self) -> f32 {
        if self.total_screen_pixels == 0 {
            0.0
        } else {
            1.0 - (self.total_dirty_pixels as f32 / self.total_screen_pixels as f32)
        }
    }
    
    /// Calculate partial update ratio
    pub fn partial_ratio(&self) -> f32 {
        let total = self.partial_updates + self.full_redraws;
        if total == 0 {
            0.0
        } else {
            self.partial_updates as f32 / total as f32
        }
    }
    
    /// Reset statistics
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
