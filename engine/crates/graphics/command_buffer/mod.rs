//! # Command Buffer System
//!
//! High-performance command batching for reducing cross-thread communication overhead.
//!
//! ## Design Goals
//!
//! 1. **Batch Processing**: Collect all draw commands within a frame, send once
//! 2. **Zero-Copy**: Use arena allocation to avoid per-command heap allocations
//! 3. **Cache-Friendly**: Commands stored contiguously for efficient iteration
//! 4. **Thread-Safe**: Lock-free command submission from JS thread
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                    JS Thread (Producer)                              │
//! │                                                                      │
//! │   RAF callback:                                                      │
//! │     ctx.fillStyle = 'red'      → CommandBuffer.push(SetFillStyle)   │
//! │     ctx.fillRect(0, 0, 100, 50) → CommandBuffer.push(FillRect)      │
//! │     ctx.drawImage(img, ...)     → CommandBuffer.push(DrawImage)      │
//! │     ... more operations ...                                          │
//! │     frame_end()                 → Submit entire CommandBuffer        │
//! │                                                                      │
//! └─────────────────────────────────────────────────────────────────────┘
//!                                    │
//!                         Channel (single message per frame)
//!                                    │
//!                                    ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                   Render Thread (Consumer)                           │
//! │                                                                      │
//! │   Receive CommandBuffer → Execute all commands → Swap buffers        │
//! │                                                                      │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```

mod arena;
mod commands;
mod encoder;

pub use commands::*;
pub use encoder::CommandEncoder;

use std::sync::atomic::{AtomicU64, Ordering};

/// Frame sequence number for ordering and debugging
static FRAME_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Get the next frame sequence number
pub fn next_frame_sequence() -> u64 {
    FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

/// A complete frame's worth of rendering commands
///
/// This is the unit of work sent from JS thread to render thread.
/// Contains all Canvas2D and WebGL commands for a single frame.
#[derive(Debug)]
pub struct CommandBuffer {
    /// Frame sequence number
    pub frame_id: u64,
    
    /// Target canvas ID (primary canvas for this frame)
    pub canvas_id: u32,
    
    /// 2D rendering commands
    pub canvas_2d_commands: Vec<Canvas2DCommand>,
    
    /// WebGL commands (if any)
    pub gl_commands: Vec<GLCommand>,
    
    /// Dirty region hints for partial update optimization
    pub dirty_regions: Vec<DirtyRect>,
    
    /// Whether this frame requires a buffer swap
    pub needs_present: bool,
}

impl CommandBuffer {
    /// Create a new empty command buffer for a frame
    pub fn new(canvas_id: u32) -> Self {
        Self {
            frame_id: next_frame_sequence(),
            canvas_id,
            canvas_2d_commands: Vec::with_capacity(256),
            gl_commands: Vec::new(),
            dirty_regions: Vec::new(),
            needs_present: true,
        }
    }
    
    /// Create with pre-allocated capacity
    pub fn with_capacity(canvas_id: u32, capacity: usize) -> Self {
        Self {
            frame_id: next_frame_sequence(),
            canvas_id,
            canvas_2d_commands: Vec::with_capacity(capacity),
            gl_commands: Vec::new(),
            dirty_regions: Vec::new(),
            needs_present: true,
        }
    }
    
    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.canvas_2d_commands.is_empty() && self.gl_commands.is_empty()
    }
    
    /// Get total command count
    pub fn command_count(&self) -> usize {
        self.canvas_2d_commands.len() + self.gl_commands.len()
    }
    
    /// Clear and reuse the buffer
    pub fn clear(&mut self) {
        self.frame_id = next_frame_sequence();
        self.canvas_2d_commands.clear();
        self.gl_commands.clear();
        self.dirty_regions.clear();
        self.needs_present = true;
    }
    
    /// Add a 2D command
    #[inline]
    pub fn push_2d(&mut self, cmd: Canvas2DCommand) {
        self.canvas_2d_commands.push(cmd);
    }
    
    /// Add a GL command
    #[inline]
    pub fn push_gl(&mut self, cmd: GLCommand) {
        self.gl_commands.push(cmd);
    }
    
    /// Mark a region as dirty (for partial update optimization)
    #[inline]
    pub fn mark_dirty(&mut self, rect: DirtyRect) {
        self.dirty_regions.push(rect);
    }
}

/// A dirty rectangle for partial update optimization
#[derive(Debug, Clone, Copy)]
pub struct DirtyRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl DirtyRect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }
    
    /// Create a full-canvas dirty rect
    pub fn full(width: f32, height: f32) -> Self {
        Self { x: 0.0, y: 0.0, width, height }
    }
    
    /// Merge with another dirty rect (union)
    pub fn union(&self, other: &DirtyRect) -> DirtyRect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        DirtyRect {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    }
}

/// Statistics about command buffer usage for profiling
#[derive(Debug, Default, Clone)]
pub struct CommandBufferStats {
    /// Total commands processed
    pub total_commands: u64,
    /// Average commands per frame
    pub avg_commands_per_frame: f32,
    /// Peak commands in a single frame
    pub peak_commands: usize,
    /// Total frames processed
    pub total_frames: u64,
}

impl CommandBufferStats {
    pub fn record_frame(&mut self, command_count: usize) {
        self.total_commands += command_count as u64;
        self.total_frames += 1;
        self.peak_commands = self.peak_commands.max(command_count);
        self.avg_commands_per_frame = self.total_commands as f32 / self.total_frames as f32;
    }
}
