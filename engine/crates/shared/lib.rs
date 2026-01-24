//! Shared types used across crates.

pub mod codec;
pub mod config;
pub mod device;
pub mod error;
pub mod op_state;
pub mod protocol;
pub mod surface;

/// Frequently used re-exports.
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
