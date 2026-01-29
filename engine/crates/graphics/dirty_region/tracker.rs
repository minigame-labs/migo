//! Dirty region tracker for partial redraws

use super::{DirtyRect, DirtyStats};

/// Maximum number of dirty rects before forcing full redraw
const MAX_DIRTY_RECTS: usize = 16;

/// Threshold for merging nearby rects (as fraction of screen)
const MERGE_THRESHOLD: f32 = 0.1;

/// Threshold for full redraw (as fraction of screen)
const FULL_REDRAW_THRESHOLD: f32 = 0.7;

/// Tracks dirty regions across frames
pub struct DirtyRegionTracker {
    /// Current frame's dirty rectangles
    dirty_rects: Vec<DirtyRect>,
    /// Screen dimensions
    screen_width: f32,
    screen_height: f32,
    /// Whether full redraw is needed
    full_redraw: bool,
    /// Statistics
    stats: DirtyStats,
    /// Whether tracking is enabled
    enabled: bool,
}

impl DirtyRegionTracker {
    /// Create a new tracker
    pub fn new(screen_width: f32, screen_height: f32) -> Self {
        Self {
            dirty_rects: Vec::with_capacity(MAX_DIRTY_RECTS),
            screen_width,
            screen_height,
            full_redraw: true, // First frame needs full redraw
            stats: DirtyStats::default(),
            enabled: true,
        }
    }
    
    /// Enable or disable tracking
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.full_redraw = true;
        }
    }
    
    /// Update screen dimensions
    pub fn set_screen_size(&mut self, width: f32, height: f32) {
        if (self.screen_width - width).abs() > 0.1 || 
           (self.screen_height - height).abs() > 0.1 {
            self.screen_width = width;
            self.screen_height = height;
            self.full_redraw = true;
        }
    }
    
    /// Mark a region as dirty
    pub fn mark_dirty(&mut self, rect: DirtyRect) {
        if !self.enabled {
            self.full_redraw = true;
            return;
        }
        
        if rect.is_empty() {
            return;
        }
        
        // Clamp to screen bounds
        let clamped = self.clamp_to_screen(&rect);
        if clamped.is_empty() {
            return;
        }
        
        // Check if we should just do full redraw
        if self.dirty_rects.len() >= MAX_DIRTY_RECTS {
            self.full_redraw = true;
            self.dirty_rects.clear();
            return;
        }
        
        // Try to merge with existing rect
        let mut merged = false;
        let merge_dist = (self.screen_width * MERGE_THRESHOLD).max(50.0);
        for existing in &mut self.dirty_rects {
            if existing.intersects(&clamped) || existing.expand(merge_dist).intersects(&clamped) {
                *existing = existing.union(&clamped);
                merged = true;
                break;
            }
        }
        
        if !merged {
            self.dirty_rects.push(clamped);
        }
        
        // Check total dirty area
        self.check_total_area();
    }
    
    /// Mark a rectangle region as dirty
    pub fn mark_rect_dirty(&mut self, x: f32, y: f32, width: f32, height: f32) {
        self.mark_dirty(DirtyRect::new(x, y, width, height));
    }
    
    /// Mark entire screen as dirty
    pub fn mark_full_redraw(&mut self) {
        self.full_redraw = true;
        self.dirty_rects.clear();
    }
    
    /// Check if full redraw is needed
    pub fn needs_full_redraw(&self) -> bool {
        self.full_redraw
    }
    
    /// Get dirty rectangles for partial redraw
    pub fn get_dirty_rects(&self) -> &[DirtyRect] {
        &self.dirty_rects
    }
    
    /// Get combined dirty region (bounding box)
    pub fn get_combined_dirty(&self) -> Option<DirtyRect> {
        if self.full_redraw {
            return Some(DirtyRect::new(0.0, 0.0, self.screen_width, self.screen_height));
        }
        
        if self.dirty_rects.is_empty() {
            return None;
        }
        
        let mut combined = self.dirty_rects[0];
        for rect in &self.dirty_rects[1..] {
            combined = combined.union(rect);
        }
        Some(combined)
    }
    
    /// Begin a new frame
    pub fn begin_frame(&mut self) {
        // Update statistics
        self.stats.total_screen_pixels += (self.screen_width * self.screen_height) as u64;
        
        if self.full_redraw {
            self.stats.full_redraws += 1;
            self.stats.total_dirty_pixels += (self.screen_width * self.screen_height) as u64;
        } else {
            self.stats.partial_updates += 1;
            let dirty_area: f32 = self.dirty_rects.iter().map(|r| r.area()).sum();
            self.stats.total_dirty_pixels += dirty_area as u64;
        }
    }
    
    /// End the frame and reset for next frame
    pub fn end_frame(&mut self) {
        self.dirty_rects.clear();
        self.full_redraw = false;
    }
    
    /// Get statistics
    pub fn stats(&self) -> &DirtyStats {
        &self.stats
    }
    
    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats.reset();
    }
    
    /// Clamp rectangle to screen bounds
    fn clamp_to_screen(&self, rect: &DirtyRect) -> DirtyRect {
        let left = rect.x.max(0.0);
        let top = rect.y.max(0.0);
        let right = rect.right().min(self.screen_width);
        let bottom = rect.bottom().min(self.screen_height);
        
        if left < right && top < bottom {
            DirtyRect::from_bounds(left, top, right, bottom)
        } else {
            DirtyRect::empty()
        }
    }

    /// Check if total dirty area exceeds threshold
    fn check_total_area(&mut self) {
        let total_dirty: f32 = self.dirty_rects.iter().map(|r| r.area()).sum();
        let screen_area = self.screen_width * self.screen_height;
        
        if total_dirty / screen_area > FULL_REDRAW_THRESHOLD {
            self.full_redraw = true;
            self.dirty_rects.clear();
        }
    }
}

impl Default for DirtyRegionTracker {
    fn default() -> Self {
        Self::new(800.0, 600.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_partial_update() {
        let mut tracker = DirtyRegionTracker::new(800.0, 600.0);
        tracker.end_frame(); // Clear initial full redraw
        
        tracker.mark_rect_dirty(100.0, 100.0, 50.0, 50.0);
        
        assert!(!tracker.needs_full_redraw());
        assert_eq!(tracker.get_dirty_rects().len(), 1);
    }
    
    #[test]
    fn test_merge_overlapping() {
        let mut tracker = DirtyRegionTracker::new(800.0, 600.0);
        tracker.end_frame();
        
        tracker.mark_rect_dirty(100.0, 100.0, 50.0, 50.0);
        tracker.mark_rect_dirty(120.0, 120.0, 50.0, 50.0);
        
        // Should be merged into one rect
        assert_eq!(tracker.get_dirty_rects().len(), 1);
    }
    
    #[test]
    fn test_full_redraw_threshold() {
        let mut tracker = DirtyRegionTracker::new(100.0, 100.0);
        tracker.end_frame();
        
        // Mark 80% of screen dirty
        tracker.mark_rect_dirty(0.0, 0.0, 100.0, 80.0);
        
        // Should trigger full redraw
        assert!(tracker.needs_full_redraw());
    }
}
