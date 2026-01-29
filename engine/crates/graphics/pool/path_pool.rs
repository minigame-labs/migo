//! Path pool for Canvas 2D operations

use femtovg::Path;
use std::sync::atomic::Ordering;
use super::PoolStats;

/// Pool for reusable Path objects
pub struct PathPool {
    /// Available paths
    available: Vec<Path>,
    /// Maximum pool size
    max_size: usize,
    /// Statistics
    stats: PoolStats,
}

impl PathPool {
    /// Default pool size
    pub const DEFAULT_SIZE: usize = 32;
    
    /// Create a new path pool
    pub fn new(max_size: usize) -> Self {
        Self {
            available: Vec::with_capacity(max_size.min(16)),
            max_size,
            stats: PoolStats::new(),
        }
    }
    
    /// Create with pre-allocated paths
    pub fn with_preallocated(max_size: usize, initial: usize) -> Self {
        let mut pool = Self::new(max_size);
        for _ in 0..initial.min(max_size) {
            pool.available.push(Path::new());
        }
        pool.stats.available.store(pool.available.len(), Ordering::Relaxed);
        pool
    }
    
    /// Acquire a path from the pool
    pub fn acquire(&mut self) -> Path {
        let path = self.available.pop();
        let hit = path.is_some();
        self.stats.record_allocation(hit);
        self.stats.available.store(self.available.len(), Ordering::Relaxed);
        path.unwrap_or_else(Path::new)
    }
    
    /// Return a path to the pool
    pub fn release(&mut self, path: Path) {
        if self.available.len() < self.max_size {
            // Note: Path::new() is used instead of clearing because
            // femtovg::Path doesn't have a clear method
            // The old path is dropped and we create a new one
            self.available.push(Path::new());
            self.stats.record_return();
            self.stats.available.store(self.available.len(), Ordering::Relaxed);
        }
        // Old path is dropped here
        drop(path);
    }
    
    /// Get statistics
    pub fn stats(&self) -> &PoolStats {
        &self.stats
    }
    
    /// Get available count
    pub fn available(&self) -> usize {
        self.available.len()
    }
    
    /// Clear the pool
    pub fn clear(&mut self) {
        self.available.clear();
        self.stats.available.store(0, Ordering::Relaxed);
    }
}

impl Default for PathPool {
    fn default() -> Self {
        Self::new(Self::DEFAULT_SIZE)
    }
}

/// Reusable path builder that returns path to pool on drop
pub struct PooledPath<'a> {
    pool: &'a mut PathPool,
    path: Option<Path>,
}

impl<'a> PooledPath<'a> {
    /// Create a new pooled path
    pub fn new(pool: &'a mut PathPool) -> Self {
        let path = Some(pool.acquire());
        Self { pool, path }
    }
    
    /// Take ownership of the path
    pub fn take(mut self) -> Path {
        self.path.take().unwrap()
    }
    
    /// Get mutable reference to the path
    pub fn path_mut(&mut self) -> &mut Path {
        self.path.as_mut().unwrap()
    }
}

impl<'a> std::ops::Deref for PooledPath<'a> {
    type Target = Path;
    
    fn deref(&self) -> &Self::Target {
        self.path.as_ref().unwrap()
    }
}

impl<'a> std::ops::DerefMut for PooledPath<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.path.as_mut().unwrap()
    }
}

impl<'a> Drop for PooledPath<'a> {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            self.pool.release(path);
        }
    }
}
