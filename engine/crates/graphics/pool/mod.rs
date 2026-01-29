//! # Object Pooling System
//!
//! Provides memory-efficient object pools for frequently allocated objects
//! to reduce heap allocations and GC pressure.
//!
//! ## Pooled Objects
//!
//! - `Path` - Reusable vector paths for Canvas 2D
//! - `CommandBuffer` - Pre-allocated command storage
//! - `BatchBuffer` - Vertex/index buffer pairs
//! - GL Buffers - VBO/IBO pools

mod path_pool;
mod buffer_pool;
mod generic_pool;

pub use path_pool::*;
pub use buffer_pool::*;
pub use generic_pool::*;

use std::sync::atomic::{AtomicUsize, Ordering};

/// Statistics for pool usage
#[derive(Debug, Default)]
pub struct PoolStats {
    /// Number of items currently in pool
    pub available: AtomicUsize,
    /// Total allocations served
    pub allocations: AtomicUsize,
    /// Hits (reused from pool)
    pub hits: AtomicUsize,
    /// Misses (new allocation)
    pub misses: AtomicUsize,
    /// Items returned to pool
    pub returns: AtomicUsize,
}

impl PoolStats {
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Record an allocation
    pub fn record_allocation(&self, hit: bool) {
        self.allocations.fetch_add(1, Ordering::Relaxed);
        if hit {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
    }
    
    /// Record a return
    pub fn record_return(&self) {
        self.returns.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Calculate hit rate (0.0 - 1.0)
    pub fn hit_rate(&self) -> f32 {
        let allocs = self.allocations.load(Ordering::Relaxed);
        let hits = self.hits.load(Ordering::Relaxed);
        if allocs == 0 {
            0.0
        } else {
            hits as f32 / allocs as f32
        }
    }
    
    /// Reset statistics
    pub fn reset(&self) {
        self.available.store(0, Ordering::Relaxed);
        self.allocations.store(0, Ordering::Relaxed);
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.returns.store(0, Ordering::Relaxed);
    }
    
    /// Get snapshot
    pub fn snapshot(&self) -> (usize, usize, usize, usize, usize) {
        (
            self.available.load(Ordering::Relaxed),
            self.allocations.load(Ordering::Relaxed),
            self.hits.load(Ordering::Relaxed),
            self.misses.load(Ordering::Relaxed),
            self.returns.load(Ordering::Relaxed),
        )
    }
}

impl Clone for PoolStats {
    fn clone(&self) -> Self {
        Self {
            available: AtomicUsize::new(self.available.load(Ordering::Relaxed)),
            allocations: AtomicUsize::new(self.allocations.load(Ordering::Relaxed)),
            hits: AtomicUsize::new(self.hits.load(Ordering::Relaxed)),
            misses: AtomicUsize::new(self.misses.load(Ordering::Relaxed)),
            returns: AtomicUsize::new(self.returns.load(Ordering::Relaxed)),
        }
    }
}
