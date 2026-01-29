//! # Render Batching System
//!
//! Provides efficient batching of draw calls to minimize GPU state changes
//! and reduce CPU overhead.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │                         Draw Command Flow                                │
//! │                                                                          │
//! │   Individual         State           Merged           Optimized          │
//! │   Draw Calls    →   Sorted     →    Batches     →    GPU Calls          │
//! │                                                                          │
//! │   [Rect A]           [A,C,E]         [Batch1]         [glDraw×3]         │
//! │   [Image B]    →     [B,D]     →     [Batch2]    →                       │
//! │   [Rect C]           [F]             [Batch3]         (vs 6 calls)       │
//! │   [Image D]                                                              │
//! │   [Rect E]                                                               │
//! │   [Text F]                                                               │
//! └─────────────────────────────────────────────────────────────────────────┘
//! ```

mod draw_batcher;
mod texture_atlas;
mod state_tracker;

pub use draw_batcher::*;
pub use texture_atlas::*;
pub use state_tracker::*;

use femtovg::Color;
use shared::protocol::render_cmd::ImageId;

/// A single draw operation that can be batched
#[derive(Debug, Clone)]
pub enum DrawOp {
    /// Fill a rectangle with solid color
    FillRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Color,
    },
    
    /// Draw an image
    DrawImage {
        image_id: ImageId,
        sx: f32,
        sy: f32,
        sw: f32,
        sh: f32,
        dx: f32,
        dy: f32,
        dw: f32,
        dh: f32,
        alpha: f32,
    },
    
    /// Draw text
    DrawText {
        text: String,
        x: f32,
        y: f32,
        color: Color,
        font_size: f32,
    },
    
    /// Custom path fill
    FillPath {
        path_id: u32,
        color: Color,
    },
    
    /// Custom path stroke
    StrokePath {
        path_id: u32,
        color: Color,
        line_width: f32,
    },
}

/// Key for sorting and batching draw operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BatchKey {
    /// Draw type (rect, image, text, path)
    pub draw_type: DrawType,
    /// Texture ID (0 for solid colors)
    pub texture_id: u32,
    /// Blend mode
    pub blend_mode: u8,
}

/// Type of draw operation
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum DrawType {
    SolidRect = 0,
    TexturedRect = 1,
    Text = 2,
    Path = 3,
}

impl DrawOp {
    /// Get the batch key for this operation
    pub fn batch_key(&self) -> BatchKey {
        match self {
            DrawOp::FillRect { .. } => BatchKey {
                draw_type: DrawType::SolidRect,
                texture_id: 0,
                blend_mode: 0,
            },
            DrawOp::DrawImage { image_id, .. } => BatchKey {
                draw_type: DrawType::TexturedRect,
                texture_id: *image_id,
                blend_mode: 0,
            },
            DrawOp::DrawText { .. } => BatchKey {
                draw_type: DrawType::Text,
                texture_id: 0,
                blend_mode: 0,
            },
            DrawOp::FillPath { .. } | DrawOp::StrokePath { .. } => BatchKey {
                draw_type: DrawType::Path,
                texture_id: 0,
                blend_mode: 0,
            },
        }
    }
    
    /// Check if this operation can be merged with another
    pub fn can_merge_with(&self, other: &DrawOp) -> bool {
        self.batch_key() == other.batch_key()
    }
}

/// Statistics for batching performance
#[derive(Debug, Clone, Default)]
pub struct BatchStats {
    /// Total draw operations submitted
    pub total_ops: u32,
    /// Number of batches after optimization
    pub batch_count: u32,
    /// Number of draw calls issued
    pub draw_calls: u32,
    /// Number of state changes
    pub state_changes: u32,
    /// Texture switches
    pub texture_switches: u32,
}

impl BatchStats {
    /// Calculate batching efficiency (higher is better)
    pub fn efficiency(&self) -> f32 {
        if self.total_ops == 0 {
            1.0
        } else {
            1.0 - (self.draw_calls as f32 / self.total_ops as f32)
        }
    }
    
    /// Reset statistics
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
