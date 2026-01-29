//! Draw call batcher for merging similar operations

use std::collections::HashMap;

use super::{BatchKey, BatchStats, DrawOp};
use femtovg::Color;
use shared::protocol::render_cmd::ImageId;

/// Vertex data for batched rendering
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct BatchVertex {
    /// Position (x, y)
    pub pos: [f32; 2],
    /// Texture coordinates (u, v)
    pub uv: [f32; 2],
    /// Color (r, g, b, a)
    pub color: [f32; 4],
}

impl BatchVertex {
    pub fn new(x: f32, y: f32, u: f32, v: f32, color: Color) -> Self {
        Self {
            pos: [x, y],
            uv: [u, v],
            color: [color.r, color.g, color.b, color.a],
        }
    }
}

/// A batch of draw operations that share the same state
#[derive(Debug)]
pub struct DrawBatch {
    /// Batch key (texture, blend mode, etc.)
    pub key: BatchKey,
    /// Vertex data
    pub vertices: Vec<BatchVertex>,
    /// Index data
    pub indices: Vec<u16>,
    /// Number of primitives
    pub primitive_count: u32,
}

impl DrawBatch {
    /// Create a new empty batch
    pub fn new(key: BatchKey) -> Self {
        Self {
            key,
            vertices: Vec::with_capacity(256),
            indices: Vec::with_capacity(512),
            primitive_count: 0,
        }
    }
    
    /// Add a quad (rectangle) to the batch
    pub fn add_quad(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        u0: f32,
        v0: f32,
        u1: f32,
        v1: f32,
        color: Color,
    ) {
        let base = self.vertices.len() as u16;
        
        // Add 4 vertices
        self.vertices.push(BatchVertex::new(x, y, u0, v0, color));
        self.vertices.push(BatchVertex::new(x + w, y, u1, v0, color));
        self.vertices.push(BatchVertex::new(x + w, y + h, u1, v1, color));
        self.vertices.push(BatchVertex::new(x, y + h, u0, v1, color));
        
        // Add 6 indices for 2 triangles
        self.indices.push(base);
        self.indices.push(base + 1);
        self.indices.push(base + 2);
        self.indices.push(base);
        self.indices.push(base + 2);
        self.indices.push(base + 3);
        
        self.primitive_count += 2;
    }
    
    /// Check if batch can accept more geometry
    pub fn can_add(&self) -> bool {
        // Limit to 16-bit indices (65535 vertices)
        self.vertices.len() < 65000
    }
    
    /// Clear the batch for reuse
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.primitive_count = 0;
    }
}

/// Batches draw operations and sorts them for efficient rendering
pub struct DrawBatcher {
    /// Pending draw operations
    pending_ops: Vec<DrawOp>,
    /// Completed batches ready for rendering
    batches: Vec<DrawBatch>,
    /// Batch pool for reuse
    batch_pool: Vec<DrawBatch>,
    /// Statistics
    stats: BatchStats,
    /// Whether to preserve order (some effects need it)
    preserve_order: bool,
    /// Maximum operations before auto-flush
    max_pending_ops: usize,
}

impl DrawBatcher {
    /// Default maximum pending operations
    const DEFAULT_MAX_OPS: usize = 4096;
    
    /// Create a new draw batcher
    pub fn new() -> Self {
        Self {
            pending_ops: Vec::with_capacity(256),
            batches: Vec::with_capacity(32),
            batch_pool: Vec::with_capacity(16),
            stats: BatchStats::default(),
            preserve_order: false,
            max_pending_ops: Self::DEFAULT_MAX_OPS,
        }
    }
    
    /// Set whether to preserve draw order
    pub fn set_preserve_order(&mut self, preserve: bool) {
        self.preserve_order = preserve;
    }
    
    /// Submit a draw operation
    pub fn submit(&mut self, op: DrawOp) {
        self.pending_ops.push(op);
        self.stats.total_ops += 1;
        
        // Auto-flush if we have too many pending ops
        if self.pending_ops.len() >= self.max_pending_ops {
            self.flush();
        }
    }
    
    /// Submit a filled rectangle
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        self.submit(DrawOp::FillRect { x, y, w, h, color });
    }
    
    /// Submit an image draw
    pub fn draw_image(
        &mut self,
        image_id: ImageId,
        sx: f32, sy: f32, sw: f32, sh: f32,
        dx: f32, dy: f32, dw: f32, dh: f32,
        alpha: f32,
    ) {
        self.submit(DrawOp::DrawImage {
            image_id, sx, sy, sw, sh, dx, dy, dw, dh, alpha,
        });
    }
    
    /// Submit multiple image draws (already batched from JS)
    pub fn draw_images(&mut self, draws: &[(ImageId, f32, f32, f32, f32, f32, f32, f32, f32, f32)]) {
        for &(image_id, sx, sy, sw, sh, dx, dy, dw, dh, alpha) in draws {
            self.submit(DrawOp::DrawImage {
                image_id, sx, sy, sw, sh, dx, dy, dw, dh, alpha,
            });
        }
    }
    
    /// Flush pending operations and create batches
    pub fn flush(&mut self) -> &[DrawBatch] {
        if self.pending_ops.is_empty() {
            return &self.batches;
        }
        
        // Clear previous batches
        for mut batch in self.batches.drain(..) {
            batch.clear();
            self.batch_pool.push(batch);
        }
        
        if self.preserve_order {
            self.batch_preserving_order();
        } else {
            self.batch_sorted();
        }
        
        self.pending_ops.clear();
        &self.batches
    }
    
    /// Batch operations while preserving their order
    fn batch_preserving_order(&mut self) {
        let mut current_key: Option<BatchKey> = None;
        let mut current_batch: Option<DrawBatch> = None;
        
        let ops = std::mem::take(&mut self.pending_ops);
        for op in &ops {
            let key = op.batch_key();

            // Check if we can continue the current batch
            let should_start_new = match (&current_key, &current_batch) {
                (Some(k), Some(b)) => *k != key || !b.can_add(),
                _ => true,
            };

            if should_start_new {
                // Save current batch
                if let Some(batch) = current_batch.take() {
                    self.batches.push(batch);
                }

                // Start new batch
                current_batch = Some(self.acquire_batch(key));
                current_key = Some(key);
                self.stats.state_changes += 1;
            }

            // Add to current batch
            if let Some(batch) = &mut current_batch {
                Self::add_op_to_batch(batch, op);
            }
        }
        drop(ops);
        
        // Don't forget the last batch
        if let Some(batch) = current_batch {
            self.batches.push(batch);
        }
        
        self.stats.batch_count = self.batches.len() as u32;
        self.stats.draw_calls = self.batches.len() as u32;
    }
    
    /// Batch operations with sorting for optimal batching
    fn batch_sorted(&mut self) {
        // Group operations by batch key
        let mut groups: HashMap<BatchKey, Vec<DrawOp>> = HashMap::new();
        
        for op in self.pending_ops.drain(..) {
            let key = op.batch_key();
            groups.entry(key).or_insert_with(Vec::new).push(op);
        }
        
        // Process each group
        let mut last_texture: Option<u32> = None;
        
        for (key, ops) in groups {
            let mut batch = self.acquire_batch(key);
            
            for op in ops {
                if !batch.can_add() {
                    // Batch is full, start a new one
                    self.batches.push(batch);
                    batch = self.acquire_batch(key);
                    self.stats.draw_calls += 1;
                }
                Self::add_op_to_batch(&mut batch, &op);
            }
            
            if batch.primitive_count > 0 {
                // Track texture switches
                if let Some(last) = last_texture {
                    if last != key.texture_id {
                        self.stats.texture_switches += 1;
                    }
                }
                last_texture = Some(key.texture_id);
                
                self.batches.push(batch);
                self.stats.draw_calls += 1;
            } else {
                self.batch_pool.push(batch);
            }
            
            self.stats.state_changes += 1;
        }
        
        self.stats.batch_count = self.batches.len() as u32;
    }
    
    /// Acquire a batch from the pool or create new
    fn acquire_batch(&mut self, key: BatchKey) -> DrawBatch {
        match self.batch_pool.pop() {
            Some(mut batch) => {
                batch.key = key;
                batch.clear();
                batch
            }
            None => DrawBatch::new(key),
        }
    }
    
    /// Add a draw operation to a batch
    fn add_op_to_batch(batch: &mut DrawBatch, op: &DrawOp) {
        match op {
            DrawOp::FillRect { x, y, w, h, color } => {
                batch.add_quad(*x, *y, *w, *h, 0.0, 0.0, 1.0, 1.0, *color);
            }
            DrawOp::DrawImage { dx, dy, dw, dh, sx, sy, sw, sh, alpha, .. } => {
                // Calculate UV coordinates (assumes normalized texture coords)
                let color = Color::rgba(255, 255, 255, (*alpha * 255.0) as u8);
                batch.add_quad(*dx, *dy, *dw, *dh, *sx, *sy, *sx + *sw, *sy + *sh, color);
            }
            DrawOp::DrawText { .. } => {
                // Text is handled separately by the text renderer
            }
            DrawOp::FillPath { .. } | DrawOp::StrokePath { .. } => {
                // Paths are handled separately
            }
        }
    }
    
    /// Get statistics for the last flush
    pub fn stats(&self) -> &BatchStats {
        &self.stats
    }
    
    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats.reset();
    }
    
    /// Clear all state
    pub fn clear(&mut self) {
        self.pending_ops.clear();
        for mut batch in self.batches.drain(..) {
            batch.clear();
            self.batch_pool.push(batch);
        }
    }
}

impl Default for DrawBatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_batch_similar_ops() {
        let mut batcher = DrawBatcher::new();
        
        // Add 10 rectangles of the same color
        let color = Color::rgb(255, 0, 0);
        for i in 0..10 {
            batcher.fill_rect(i as f32 * 10.0, 0.0, 10.0, 10.0, color);
        }
        
        let batches = batcher.flush();
        
        // Should be batched into 1 batch
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].primitive_count, 20); // 10 quads * 2 triangles
    }
    
    #[test]
    fn test_preserve_order() {
        let mut batcher = DrawBatcher::new();
        batcher.set_preserve_order(true);
        
        // Alternating colors forces separate batches when preserving order
        let red = Color::rgb(255, 0, 0);
        let blue = Color::rgb(0, 0, 255);
        
        batcher.fill_rect(0.0, 0.0, 10.0, 10.0, red);
        // Image in between
        batcher.draw_image(1, 0.0, 0.0, 32.0, 32.0, 10.0, 0.0, 10.0, 10.0, 1.0);
        batcher.fill_rect(20.0, 0.0, 10.0, 10.0, blue);
        
        let batches = batcher.flush();
        
        // Should be 3 batches due to state changes
        assert_eq!(batches.len(), 3);
    }
}
