//! Resource management for lifecycle transitions
//!
//! Handles releasing and restoring GPU resources during app lifecycle changes.

use std::collections::HashMap;

use tracing::{info, debug};

/// Resource state for lifecycle transitions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceState {
    /// Resource is loaded and ready
    Loaded,
    /// Resource is released (GPU memory freed)
    Released,
    /// Resource needs to be restored
    NeedsRestore,
}

/// Information about a managed resource
#[derive(Debug)]
pub struct ResourceInfo {
    /// Resource type
    pub resource_type: ResourceType,
    /// Current state
    pub state: ResourceState,
    /// Size in bytes (for memory tracking)
    pub size_bytes: usize,
    /// Priority (higher = keep longer)
    pub priority: u8,
}

/// Types of managed resources
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceType {
    Texture,
    Buffer,
    Shader,
    Framebuffer,
    RenderTarget,
}

/// Manages GPU resources during lifecycle transitions
pub struct LifecycleResourceManager {
    /// Tracked resources
    resources: HashMap<u64, ResourceInfo>,
    /// Next resource ID
    next_id: u64,
    /// Total GPU memory used
    total_memory: usize,
    /// Memory limit for background mode
    background_memory_limit: usize,
    /// Currently in reduced memory mode
    reduced_mode: bool,
}

impl LifecycleResourceManager {
    /// Default background memory limit (32 MB)
    const DEFAULT_BACKGROUND_LIMIT: usize = 32 * 1024 * 1024;
    
    /// Create a new resource manager
    pub fn new() -> Self {
        Self {
            resources: HashMap::with_capacity(128),
            next_id: 1,
            total_memory: 0,
            background_memory_limit: Self::DEFAULT_BACKGROUND_LIMIT,
            reduced_mode: false,
        }
    }
    
    /// Set memory limit for background mode
    pub fn set_background_memory_limit(&mut self, limit: usize) {
        self.background_memory_limit = limit;
    }
    
    /// Register a new resource
    pub fn register(
        &mut self,
        resource_type: ResourceType,
        size_bytes: usize,
        priority: u8,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        
        self.resources.insert(id, ResourceInfo {
            resource_type,
            state: ResourceState::Loaded,
            size_bytes,
            priority,
        });
        
        self.total_memory += size_bytes;
        debug!("Resource registered: id={}, type={:?}, size={}KB", 
               id, resource_type, size_bytes / 1024);
        
        id
    }
    
    /// Unregister a resource
    pub fn unregister(&mut self, id: u64) {
        if let Some(info) = self.resources.remove(&id) {
            if info.state == ResourceState::Loaded {
                self.total_memory = self.total_memory.saturating_sub(info.size_bytes);
            }
            debug!("Resource unregistered: id={}", id);
        }
    }
    
    /// Mark a resource as released
    pub fn mark_released(&mut self, id: u64) {
        if let Some(info) = self.resources.get_mut(&id) {
            if info.state == ResourceState::Loaded {
                self.total_memory = self.total_memory.saturating_sub(info.size_bytes);
            }
            info.state = ResourceState::Released;
        }
    }
    
    /// Mark a resource as loaded
    pub fn mark_loaded(&mut self, id: u64) {
        if let Some(info) = self.resources.get_mut(&id) {
            if info.state != ResourceState::Loaded {
                self.total_memory += info.size_bytes;
            }
            info.state = ResourceState::Loaded;
        }
    }
    
    /// Enter reduced memory mode (background)
    pub fn enter_reduced_mode(&mut self) -> Vec<u64> {
        if self.reduced_mode {
            return Vec::new();
        }
        
        self.reduced_mode = true;
        info!("Entering reduced memory mode, current usage: {}KB, limit: {}KB",
              self.total_memory / 1024, self.background_memory_limit / 1024);
        
        // Find resources to release (lowest priority first)
        let mut to_release = Vec::new();
        
        if self.total_memory > self.background_memory_limit {
            let mut candidates: Vec<_> = self.resources.iter()
                .filter(|(_, info)| info.state == ResourceState::Loaded)
                .collect();
            
            // Sort by priority (lowest first)
            candidates.sort_by_key(|(_, info)| info.priority);
            
            let mut current = self.total_memory;
            for (id, info) in candidates {
                if current <= self.background_memory_limit {
                    break;
                }
                to_release.push(*id);
                current -= info.size_bytes;
            }
        }
        
        // Mark them for release
        for id in &to_release {
            if let Some(info) = self.resources.get_mut(id) {
                info.state = ResourceState::NeedsRestore;
            }
        }
        
        info!("Marked {} resources for release", to_release.len());
        to_release
    }
    
    /// Exit reduced memory mode (return to foreground)
    pub fn exit_reduced_mode(&mut self) -> Vec<u64> {
        if !self.reduced_mode {
            return Vec::new();
        }
        
        self.reduced_mode = false;
        info!("Exiting reduced memory mode");
        
        // Find resources that need restoration
        let to_restore: Vec<u64> = self.resources.iter()
            .filter(|(_, info)| info.state == ResourceState::NeedsRestore)
            .map(|(id, _)| *id)
            .collect();
        
        info!("Need to restore {} resources", to_restore.len());
        to_restore
    }
    
    /// Release all resources (entering suspended state)
    pub fn release_all(&mut self) -> Vec<u64> {
        info!("Releasing all resources");
        
        let to_release: Vec<u64> = self.resources.iter()
            .filter(|(_, info)| info.state == ResourceState::Loaded)
            .map(|(id, _)| *id)
            .collect();
        
        for id in &to_release {
            if let Some(info) = self.resources.get_mut(id) {
                info.state = ResourceState::NeedsRestore;
            }
        }
        
        self.total_memory = 0;
        to_release
    }
    
    /// Get all resources that need restoration
    pub fn get_needs_restore(&self) -> Vec<u64> {
        self.resources.iter()
            .filter(|(_, info)| info.state == ResourceState::NeedsRestore)
            .map(|(id, _)| *id)
            .collect()
    }
    
    /// Get total memory usage
    pub fn total_memory(&self) -> usize {
        self.total_memory
    }
    
    /// Get resource count
    pub fn resource_count(&self) -> usize {
        self.resources.len()
    }
    
    /// Get resource info
    pub fn get_resource(&self, id: u64) -> Option<&ResourceInfo> {
        self.resources.get(&id)
    }
    
    /// Clear all resources
    pub fn clear(&mut self) {
        self.resources.clear();
        self.total_memory = 0;
        self.reduced_mode = false;
    }
}

impl Default for LifecycleResourceManager {
    fn default() -> Self {
        Self::new()
    }
}
