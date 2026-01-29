//! Simple arena allocator for command buffer memory management
//!
//! This provides fast bump allocation for temporary frame data,
//! with the entire arena reset at the end of each frame.

use std::cell::UnsafeCell;

/// A simple bump allocator for frame-scoped allocations
///
/// All allocations are freed at once when reset() is called,
/// making it extremely fast for per-frame temporary data.
pub struct Arena {
    buffer: UnsafeCell<Vec<u8>>,
    offset: usize,
}

impl Arena {
    /// Create a new arena with the given capacity
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: UnsafeCell::new(vec![0u8; capacity]),
            offset: 0,
        }
    }
    
    /// Allocate space for T and return a mutable reference
    ///
    /// # Safety
    /// The returned reference is only valid until reset() is called
    #[allow(dead_code)]
    pub fn alloc<T: Sized>(&mut self) -> Option<&mut T> {
        let size = std::mem::size_of::<T>();
        let align = std::mem::align_of::<T>();
        
        // Align the offset
        let aligned_offset = (self.offset + align - 1) & !(align - 1);
        
        let buffer = unsafe { &mut *self.buffer.get() };
        
        if aligned_offset + size > buffer.len() {
            return None;
        }
        
        self.offset = aligned_offset + size;
        
        // Safety: We've ensured the memory is available and aligned
        let ptr = buffer.as_mut_ptr().wrapping_add(aligned_offset) as *mut T;
        Some(unsafe { &mut *ptr })
    }
    
    /// Allocate a slice of bytes
    #[allow(dead_code)]
    pub fn alloc_bytes(&mut self, size: usize) -> Option<&mut [u8]> {
        let buffer = unsafe { &mut *self.buffer.get() };
        
        if self.offset + size > buffer.len() {
            return None;
        }
        
        let start = self.offset;
        self.offset += size;
        
        Some(&mut buffer[start..self.offset])
    }
    
    /// Reset the arena, freeing all allocations
    pub fn reset(&mut self) {
        self.offset = 0;
    }
    
    /// Get the current usage
    #[allow(dead_code)]
    pub fn used(&self) -> usize {
        self.offset
    }
    
    /// Get the total capacity
    #[allow(dead_code)]
    pub fn capacity(&self) -> usize {
        unsafe { &*self.buffer.get() }.len()
    }
}

// Safety: Arena is Send because all access is through &mut self
unsafe impl Send for Arena {}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_arena_basic() {
        let mut arena = Arena::new(1024);
        
        let a: &mut u32 = arena.alloc().unwrap();
        *a = 42;
        
        let b: &mut u64 = arena.alloc().unwrap();
        *b = 123;
        
        assert!(*a == 42);
        assert!(*b == 123);
        
        arena.reset();
        assert!(arena.used() == 0);
    }
}
