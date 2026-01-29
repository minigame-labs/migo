//! # Application Lifecycle Management
//!
//! Handles application state transitions (foreground/background/suspended)
//! and manages resources accordingly.
//!
//! ## State Diagram
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │                      Application Lifecycle                               │
//! │                                                                          │
//! │   ┌──────────┐      onPause()      ┌──────────────┐                     │
//! │   │  Active  │ ─────────────────→  │  Background  │                     │
//! │   │ (60 FPS) │                     │   (1 FPS)    │                     │
//! │   └──────────┘                     └──────────────┘                     │
//! │        ↑                                   │                             │
//! │        │ onResume()                        │ onSurfaceDestroyed()       │
//! │        │                                   ↓                             │
//! │   ┌──────────┐      onStop()       ┌──────────────┐                     │
//! │   │ Starting │ ←───────────────    │  Suspended   │                     │
//! │   │          │                     │  (0 FPS)     │                     │
//! │   └──────────┘                     └──────────────┘                     │
//! │                                                                          │
//! │   Active:     Full rendering, all resources loaded                       │
//! │   Background: Reduced rendering, GPU resources kept                      │
//! │   Suspended:  No rendering, minimal GPU resources                        │
//! └─────────────────────────────────────────────────────────────────────────┘
//! ```

mod app_state;
mod resource_manager;
mod surface_manager;

pub use app_state::*;
pub use resource_manager::*;
pub use surface_manager::*;

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// Application lifecycle state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AppLifecycleState {
    /// App is not yet initialized
    Uninitialized = 0,
    /// App is starting up (loading resources)
    Starting = 1,
    /// App is in foreground, fully active
    Active = 2,
    /// App is in background but surface is valid
    Background = 3,
    /// App is suspended (surface destroyed)
    Suspended = 4,
    /// App is shutting down
    Stopping = 5,
}

impl From<u8> for AppLifecycleState {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::Uninitialized,
            1 => Self::Starting,
            2 => Self::Active,
            3 => Self::Background,
            4 => Self::Suspended,
            5 => Self::Stopping,
            _ => Self::Uninitialized,
        }
    }
}

/// Shared lifecycle state accessible across threads
#[derive(Clone)]
pub struct SharedLifecycleState {
    state: Arc<AtomicU8>,
}

impl SharedLifecycleState {
    pub fn new() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(AppLifecycleState::Uninitialized as u8)),
        }
    }
    
    /// Get current state
    pub fn get(&self) -> AppLifecycleState {
        AppLifecycleState::from(self.state.load(Ordering::Acquire))
    }
    
    /// Set state
    pub fn set(&self, state: AppLifecycleState) {
        self.state.store(state as u8, Ordering::Release);
    }
    
    /// Check if app is active (foreground)
    pub fn is_active(&self) -> bool {
        self.get() == AppLifecycleState::Active
    }
    
    /// Check if app should render
    pub fn should_render(&self) -> bool {
        matches!(self.get(), AppLifecycleState::Active | AppLifecycleState::Background)
    }
    
    /// Check if app is suspended (no surface)
    pub fn is_suspended(&self) -> bool {
        matches!(self.get(), AppLifecycleState::Suspended | AppLifecycleState::Stopping)
    }
}

impl Default for SharedLifecycleState {
    fn default() -> Self {
        Self::new()
    }
}

/// Configuration for different lifecycle states
#[derive(Debug, Clone)]
pub struct LifecycleConfig {
    /// Target FPS when active
    pub active_fps: u32,
    /// Target FPS when in background
    pub background_fps: u32,
    /// Whether to release GPU resources when suspended
    pub release_resources_on_suspend: bool,
    /// Whether to keep audio playing in background
    pub audio_in_background: bool,
    /// Timeout before entering deep sleep (ms)
    pub background_timeout_ms: u64,
}

impl Default for LifecycleConfig {
    fn default() -> Self {
        Self {
            active_fps: 60,
            background_fps: 5,
            release_resources_on_suspend: true,
            audio_in_background: true,
            background_timeout_ms: 5000,
        }
    }
}

/// Lifecycle event for notification
#[derive(Debug, Clone)]
pub enum LifecycleEvent {
    /// App became active (resumed)
    Activated,
    /// App went to background
    Deactivated,
    /// Surface was created
    SurfaceCreated { width: u32, height: u32 },
    /// Surface was resized
    SurfaceResized { width: u32, height: u32 },
    /// Surface was destroyed
    SurfaceDestroyed,
    /// Low memory warning
    LowMemory,
    /// Configuration changed (orientation, etc.)
    ConfigChanged,
}

/// Listener for lifecycle events
pub trait LifecycleListener: Send + Sync {
    /// Called when lifecycle event occurs
    fn on_lifecycle_event(&self, event: LifecycleEvent);
}
