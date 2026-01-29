//! Application state machine for lifecycle management

use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::{AppLifecycleState, LifecycleConfig, LifecycleEvent, LifecycleListener, SharedLifecycleState};
use tracing::{info, warn};

/// Manages application lifecycle state transitions
pub struct AppStateManager {
    /// Current state
    state: SharedLifecycleState,
    /// Configuration
    config: LifecycleConfig,
    /// Time entered background
    background_start: Option<Instant>,
    /// Registered listeners
    listeners: Vec<Arc<dyn LifecycleListener>>,
    /// Surface state
    surface_valid: bool,
    /// Last surface dimensions
    surface_size: (u32, u32),
}

impl AppStateManager {
    /// Create a new app state manager
    pub fn new(config: LifecycleConfig) -> Self {
        Self {
            state: SharedLifecycleState::new(),
            config,
            background_start: None,
            listeners: Vec::new(),
            surface_valid: false,
            surface_size: (0, 0),
        }
    }
    
    /// Get shared state handle (for other threads)
    pub fn shared_state(&self) -> SharedLifecycleState {
        self.state.clone()
    }
    
    /// Register a lifecycle listener
    pub fn add_listener(&mut self, listener: Arc<dyn LifecycleListener>) {
        self.listeners.push(listener);
    }
    
    /// Get current state
    pub fn current_state(&self) -> AppLifecycleState {
        self.state.get()
    }
    
    /// Get target FPS for current state
    pub fn target_fps(&self) -> u32 {
        match self.state.get() {
            AppLifecycleState::Active => self.config.active_fps,
            AppLifecycleState::Background => self.config.background_fps,
            _ => 0,
        }
    }
    
    /// Called when app is resumed (comes to foreground)
    pub fn on_resume(&mut self) {
        let old_state = self.state.get();
        info!("AppStateManager: onResume (from {:?})", old_state);
        
        self.background_start = None;
        
        if self.surface_valid {
            self.state.set(AppLifecycleState::Active);
            self.notify(LifecycleEvent::Activated);
        } else {
            self.state.set(AppLifecycleState::Starting);
        }
    }
    
    /// Called when app is paused (goes to background)
    pub fn on_pause(&mut self) {
        let old_state = self.state.get();
        info!("AppStateManager: onPause (from {:?})", old_state);
        
        self.background_start = Some(Instant::now());
        
        if self.surface_valid {
            self.state.set(AppLifecycleState::Background);
            self.notify(LifecycleEvent::Deactivated);
        }
    }
    
    /// Called when surface is created
    pub fn on_surface_created(&mut self, width: u32, height: u32) {
        info!("AppStateManager: onSurfaceCreated {}x{}", width, height);
        
        self.surface_valid = true;
        self.surface_size = (width, height);
        
        let current = self.state.get();
        match current {
            AppLifecycleState::Suspended | AppLifecycleState::Starting => {
                self.state.set(AppLifecycleState::Active);
                self.notify(LifecycleEvent::SurfaceCreated { width, height });
                self.notify(LifecycleEvent::Activated);
            }
            AppLifecycleState::Background => {
                // Surface recreated while in background, stay in background
                self.notify(LifecycleEvent::SurfaceCreated { width, height });
            }
            _ => {
                self.notify(LifecycleEvent::SurfaceCreated { width, height });
            }
        }
    }
    
    /// Called when surface is resized
    pub fn on_surface_changed(&mut self, width: u32, height: u32) {
        if self.surface_size == (width, height) {
            return; // No change
        }
        
        info!("AppStateManager: onSurfaceChanged {}x{}", width, height);
        self.surface_size = (width, height);
        self.notify(LifecycleEvent::SurfaceResized { width, height });
    }
    
    /// Called when surface is destroyed
    pub fn on_surface_destroyed(&mut self) {
        info!("AppStateManager: onSurfaceDestroyed");
        
        self.surface_valid = false;
        self.notify(LifecycleEvent::SurfaceDestroyed);
        
        // Move to suspended state - but DO NOT exit threads!
        self.state.set(AppLifecycleState::Suspended);
    }
    
    /// Called on low memory warning
    pub fn on_low_memory(&mut self) {
        warn!("AppStateManager: onLowMemory");
        self.notify(LifecycleEvent::LowMemory);
    }
    
    /// Called when app is being destroyed
    pub fn on_destroy(&mut self) {
        info!("AppStateManager: onDestroy");
        self.state.set(AppLifecycleState::Stopping);
    }
    
    /// Check if should enter deep sleep (extended background)
    pub fn should_deep_sleep(&self) -> bool {
        if let Some(start) = self.background_start {
            start.elapsed().as_millis() > self.config.background_timeout_ms as u128
        } else {
            false
        }
    }
    
    /// Check if surface is currently valid
    pub fn has_valid_surface(&self) -> bool {
        self.surface_valid
    }
    
    /// Get current surface size
    pub fn surface_size(&self) -> (u32, u32) {
        self.surface_size
    }
    
    /// Notify all listeners
    fn notify(&self, event: LifecycleEvent) {
        for listener in &self.listeners {
            listener.on_lifecycle_event(event.clone());
        }
    }
}

/// Thread-safe wrapper for AppStateManager
pub struct ThreadSafeAppState {
    inner: Arc<Mutex<AppStateManager>>,
    shared: SharedLifecycleState,
}

impl ThreadSafeAppState {
    pub fn new(config: LifecycleConfig) -> Self {
        let manager = AppStateManager::new(config);
        let shared = manager.shared_state();
        Self {
            inner: Arc::new(Mutex::new(manager)),
            shared,
        }
    }
    
    /// Get shared state for lock-free reads
    pub fn shared(&self) -> SharedLifecycleState {
        self.shared.clone()
    }
    
    /// Execute with lock
    pub fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut AppStateManager) -> R,
    {
        let mut guard = self.inner.lock().unwrap();
        f(&mut guard)
    }
    
    /// Quick state check (lock-free)
    pub fn current_state(&self) -> AppLifecycleState {
        self.shared.get()
    }
    
    /// Quick active check (lock-free)
    pub fn is_active(&self) -> bool {
        self.shared.is_active()
    }
    
    /// Quick render check (lock-free)
    pub fn should_render(&self) -> bool {
        self.shared.should_render()
    }
}

impl Clone for ThreadSafeAppState {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            shared: self.shared.clone(),
        }
    }
}
