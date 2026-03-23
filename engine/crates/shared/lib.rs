//! # Shared Types Library
//!
//! This crate provides shared types, utilities, and protocols used across the Migo engine.
//! It serves as the common foundation for inter-crate communication and shared abstractions.
//!
//! ## Module Overview
//!
//! - [`codec`]: Encoding/decoding utilities for various text formats (UTF-8, Base64, Hex, etc.)
//! - [`config`]: Engine initialization configuration options
//! - [`device`]: Device and system settings abstractions
//! - [`error`]: Unified error handling with [`ErrorCode`] and [`EngineError`](error::EngineError)
//! - [`op_state`]: Shared operational state for Deno extensions
//! - [`protocol`]: Inter-service communication protocols (Host, IO, Render, Audio commands)
//! - [`surface`]: Platform-agnostic surface and window abstractions
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use shared::prelude::*;
//!
//! // Create initialization options
//! let options = InitOptions::new("/tmp/app", 2.0);
//!
//! // Handle errors with ErrorCode
//! fn example_operation() -> shared::error::EngineResult<()> {
//!     shared::bail!(ErrorCode::NotFound, "resource not available");
//! }
//! ```
//!
//! ## Protocol Architecture
//!
//! The engine uses a message-passing architecture with typed commands:
//!
//! - `HostCommand`: Commands sent to the JS runtime thread
//! - `RenderCommand`: Graphics/rendering operations
//! - `IOCmd`: File system and network I/O operations
//! - `AudioCmd`: Audio playback and processing commands
//!
//! Each command type flows through dedicated channels, enabling concurrent
//! processing across specialized threads (main, render, audio, IO).

pub mod channel;
pub mod cjs_compat;
pub mod codec;
pub mod config;
pub mod console_log;
pub mod device;
pub mod error;
pub mod op_state;
pub mod protocol;
pub mod services;
pub mod stats;
pub mod surface;
pub mod vfs;

#[cfg(test)]
pub mod test_support;

/// Frequently used re-exports for convenient imports.
///
/// # Example
///
/// ```rust,ignore
/// use shared::prelude::*;
///
/// let options = InitOptions::new("/tmp", 1.5);
/// let window = WindowInfo::default();
/// ```
pub mod prelude {
    pub use crate::config::InitOptions;
    pub use crate::device::SystemSettings;
    pub use crate::error::{ErrorCode, io_error_to_error_code};
    pub use crate::surface::{SafeArea, Surface, SurfaceRef, WindowInfo};
}

/// Top-level stable re-exports (keep minimal to avoid API churn).
pub use config::InitOptions;
pub use device::SystemSettings;
pub use error::{ErrorCode, io_error_to_error_code};
pub use surface::{SafeArea, Surface, SurfaceRef, WindowInfo};

/// Protocol types are intentionally namespaced.
/// Prefer `use shared::protocol::...` in downstream crates.
pub use protocol::{
    host_cmd::HostCommand,
    io_cmd::{IOCmd, IOCmdResp},
    render_cmd::RenderCommand,
};
