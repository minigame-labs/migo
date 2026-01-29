//! Generic object pool implementation

use std::sync::atomic::Ordering;
use super::PoolStats;

/// Trait for resettable pooled objects
pub trait Resettable {
    /// Reset the object to initial state for reuse
    fn reset(&mut self);
}

/// Generic object pool
pub struct ObjectPool<T: Resettable + Default> {
    /// Available objects
    available: Vec<T>,
    /// Maximum pool size
    max_size: usize,
    /// Statistics
    stats: PoolStats,
}

impl<T: Resettable + Default> ObjectPool<T> {
    /// Create a new pool with default capacity
    pub fn new(max_size: usize) -> Self {
        Self {
            available: Vec::with_capacity(max_size.min(16)),
            max_size,
            stats: PoolStats::new(),
        }
    }
    
    /// Create a pool with pre-allocated objects
    pub fn with_preallocated(max_size: usize, initial: usize) -> Self {
        let mut pool = Self::new(max_size);
        for _ in 0..initial.min(max_size) {
            pool.available.push(T::default());
        }
        pool.stats.available.store(pool.available.len(), Ordering::Relaxed);
        pool
    }
    
    /// Acquire an object from the pool or create new
    pub fn acquire(&mut self) -> T {
        let obj = self.available.pop();
        let hit = obj.is_some();
        self.stats.record_allocation(hit);
        self.stats.available.store(self.available.len(), Ordering::Relaxed);
        obj.unwrap_or_default()
    }
    
    /// Return an object to the pool
    pub fn release(&mut self, mut obj: T) {
        if self.available.len() < self.max_size {
            obj.reset();
            self.available.push(obj);
            self.stats.record_return();
            self.stats.available.store(self.available.len(), Ordering::Relaxed);
        }
        // If pool is full, object is dropped
    }
    
    /// Get pool statistics
    pub fn stats(&self) -> &PoolStats {
        &self.stats
    }
    
    /// Get number of available objects
    pub fn available(&self) -> usize {
        self.available.len()
    }
    
    /// Clear the pool
    pub fn clear(&mut self) {
        self.available.clear();
        self.stats.available.store(0, Ordering::Relaxed);
    }
    
    /// Shrink pool to target size
    pub fn shrink_to(&mut self, target: usize) {
        while self.available.len() > target {
            self.available.pop();
        }
        self.stats.available.store(self.available.len(), Ordering::Relaxed);
    }
}

/// Scoped object that returns to pool on drop
pub struct Pooled<'a, T: Resettable + Default> {
    pool: &'a mut ObjectPool<T>,
    value: Option<T>,
}

impl<'a, T: Resettable + Default> Pooled<'a, T> {
    /// Create a new pooled object
    pub fn new(pool: &'a mut ObjectPool<T>) -> Self {
        let value = Some(pool.acquire());
        Self { pool, value }
    }
    
    /// Take ownership of the value (prevents return to pool)
    pub fn take(mut self) -> T {
        self.value.take().unwrap()
    }
}

impl<'a, T: Resettable + Default> std::ops::Deref for Pooled<'a, T> {
    type Target = T;
    
    fn deref(&self) -> &Self::Target {
        self.value.as_ref().unwrap()
    }
}

impl<'a, T: Resettable + Default> std::ops::DerefMut for Pooled<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().unwrap()
    }
}

impl<'a, T: Resettable + Default> Drop for Pooled<'a, T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            self.pool.release(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[derive(Default)]
    struct TestObject {
        value: i32,
    }
    
    impl Resettable for TestObject {
        fn reset(&mut self) {
            self.value = 0;
        }
    }
    
    #[test]
    fn test_acquire_release() {
        let mut pool = ObjectPool::<TestObject>::new(10);
        
        let mut obj = pool.acquire();
        obj.value = 42;
        pool.release(obj);
        
        assert_eq!(pool.available(), 1);
        
        let obj2 = pool.acquire();
        assert_eq!(obj2.value, 0); // Should be reset
        assert_eq!(pool.available(), 0);
    }
    
    #[test]
    fn test_pool_max_size() {
        let mut pool = ObjectPool::<TestObject>::new(2);
        
        pool.release(TestObject::default());
        pool.release(TestObject::default());
        pool.release(TestObject::default()); // Should be dropped
        
        assert_eq!(pool.available(), 2);
    }
    
    #[test]
    fn test_hit_rate() {
        let mut pool = ObjectPool::<TestObject>::with_preallocated(10, 5);
        
        // 5 hits
        for _ in 0..5 {
            let obj = pool.acquire();
            pool.release(obj);
        }
        
        // All should be hits (reused from pool)
        assert!((pool.stats().hit_rate() - 1.0).abs() < 0.01);
    }
}
