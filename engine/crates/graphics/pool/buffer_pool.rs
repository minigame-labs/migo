//! WebGL buffer pool for VBO/IBO reuse

use glow::HasContext;
use std::sync::atomic::Ordering;
use super::PoolStats;

/// A pooled buffer entry
#[derive(Debug)]
struct BufferEntry {
    buffer: glow::NativeBuffer,
    capacity: usize,
    target: u32,
}

/// Pool for WebGL buffers (VBO/IBO)
pub struct BufferPool {
    /// Available vertex buffers
    vertex_buffers: Vec<BufferEntry>,
    /// Available index buffers
    index_buffers: Vec<BufferEntry>,
    /// Maximum pool size per type
    max_size: usize,
    /// Statistics
    stats: PoolStats,
}

impl BufferPool {
    /// Default pool size
    pub const DEFAULT_SIZE: usize = 16;
    
    /// Create a new buffer pool
    pub fn new(max_size: usize) -> Self {
        Self {
            vertex_buffers: Vec::with_capacity(max_size),
            index_buffers: Vec::with_capacity(max_size),
            max_size,
            stats: PoolStats::new(),
        }
    }
    
    /// Acquire a vertex buffer of at least the specified capacity
    pub fn acquire_vertex_buffer(
        &mut self,
        gl: &glow::Context,
        min_capacity: usize,
    ) -> Option<glow::NativeBuffer> {
        // Find a buffer with sufficient capacity
        let idx = self.vertex_buffers.iter()
            .position(|e| e.capacity >= min_capacity);
        
        if let Some(idx) = idx {
            let entry = self.vertex_buffers.remove(idx);
            self.stats.record_allocation(true);
            self.stats.available.store(
                self.vertex_buffers.len() + self.index_buffers.len(),
                Ordering::Relaxed
            );
            return Some(entry.buffer);
        }
        
        // Create new buffer
        let buffer = unsafe { gl.create_buffer().ok()? };
        self.stats.record_allocation(false);
        Some(buffer)
    }
    
    /// Acquire an index buffer of at least the specified capacity
    pub fn acquire_index_buffer(
        &mut self,
        gl: &glow::Context,
        min_capacity: usize,
    ) -> Option<glow::NativeBuffer> {
        let idx = self.index_buffers.iter()
            .position(|e| e.capacity >= min_capacity);
        
        if let Some(idx) = idx {
            let entry = self.index_buffers.remove(idx);
            self.stats.record_allocation(true);
            self.stats.available.store(
                self.vertex_buffers.len() + self.index_buffers.len(),
                Ordering::Relaxed
            );
            return Some(entry.buffer);
        }
        
        let buffer = unsafe { gl.create_buffer().ok()? };
        self.stats.record_allocation(false);
        Some(buffer)
    }
    
    /// Return a vertex buffer to the pool
    pub fn release_vertex_buffer(
        &mut self,
        gl: &glow::Context,
        buffer: glow::NativeBuffer,
        capacity: usize,
    ) {
        if self.vertex_buffers.len() < self.max_size {
            self.vertex_buffers.push(BufferEntry {
                buffer,
                capacity,
                target: glow::ARRAY_BUFFER,
            });
            // Sort by capacity for better matching
            self.vertex_buffers.sort_by_key(|e| e.capacity);
            self.stats.record_return();
            self.stats.available.store(
                self.vertex_buffers.len() + self.index_buffers.len(),
                Ordering::Relaxed
            );
        } else {
            unsafe { gl.delete_buffer(buffer); }
        }
    }
    
    /// Return an index buffer to the pool
    pub fn release_index_buffer(
        &mut self,
        gl: &glow::Context,
        buffer: glow::NativeBuffer,
        capacity: usize,
    ) {
        if self.index_buffers.len() < self.max_size {
            self.index_buffers.push(BufferEntry {
                buffer,
                capacity,
                target: glow::ELEMENT_ARRAY_BUFFER,
            });
            self.index_buffers.sort_by_key(|e| e.capacity);
            self.stats.record_return();
            self.stats.available.store(
                self.vertex_buffers.len() + self.index_buffers.len(),
                Ordering::Relaxed
            );
        } else {
            unsafe { gl.delete_buffer(buffer); }
        }
    }
    
    /// Get statistics
    pub fn stats(&self) -> &PoolStats {
        &self.stats
    }
    
    /// Get total available buffers
    pub fn available(&self) -> usize {
        self.vertex_buffers.len() + self.index_buffers.len()
    }
    
    /// Clear and destroy all pooled buffers
    pub fn clear(&mut self, gl: &glow::Context) {
        for entry in self.vertex_buffers.drain(..) {
            unsafe { gl.delete_buffer(entry.buffer); }
        }
        for entry in self.index_buffers.drain(..) {
            unsafe { gl.delete_buffer(entry.buffer); }
        }
        self.stats.available.store(0, Ordering::Relaxed);
    }
}

impl Default for BufferPool {
    fn default() -> Self {
        Self::new(Self::DEFAULT_SIZE)
    }
}

/// Managed buffer that tracks its capacity and can be returned to pool
pub struct ManagedBuffer {
    pub buffer: glow::NativeBuffer,
    pub capacity: usize,
    pub target: u32,
}

impl ManagedBuffer {
    /// Create a new managed vertex buffer
    pub fn new_vertex(gl: &glow::Context, initial_capacity: usize) -> Option<Self> {
        let buffer = unsafe { gl.create_buffer().ok()? };
        
        // Allocate initial storage
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
            gl.buffer_data_size(
                glow::ARRAY_BUFFER,
                initial_capacity as i32,
                glow::DYNAMIC_DRAW,
            );
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
        
        Some(Self {
            buffer,
            capacity: initial_capacity,
            target: glow::ARRAY_BUFFER,
        })
    }
    
    /// Create a new managed index buffer
    pub fn new_index(gl: &glow::Context, initial_capacity: usize) -> Option<Self> {
        let buffer = unsafe { gl.create_buffer().ok()? };
        
        unsafe {
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(buffer));
            gl.buffer_data_size(
                glow::ELEMENT_ARRAY_BUFFER,
                initial_capacity as i32,
                glow::DYNAMIC_DRAW,
            );
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);
        }
        
        Some(Self {
            buffer,
            capacity: initial_capacity,
            target: glow::ELEMENT_ARRAY_BUFFER,
        })
    }
    
    /// Upload data, reallocating if necessary
    pub fn upload(&mut self, gl: &glow::Context, data: &[u8]) {
        unsafe {
            gl.bind_buffer(self.target, Some(self.buffer));
            
            if data.len() > self.capacity {
                // Need to reallocate
                self.capacity = data.len().next_power_of_two();
                gl.buffer_data_u8_slice(self.target, data, glow::DYNAMIC_DRAW);
            } else {
                // Just update
                gl.buffer_sub_data_u8_slice(self.target, 0, data);
            }
        }
    }
    
    /// Destroy the buffer
    pub fn destroy(self, gl: &glow::Context) {
        unsafe { gl.delete_buffer(self.buffer); }
    }
}
