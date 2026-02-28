pub type HostId = i32;

mod host;
mod loader;

pub mod registry;
pub mod thread;
pub mod vsync;

#[cfg(feature = "v8-limits")]
pub mod watchdog;

pub use registry::{send_command_to_host, shutdown_host};
pub use thread::spawn_host_thread;
