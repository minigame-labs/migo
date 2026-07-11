pub type HostId = i32;

pub(crate) mod code_cache;
mod host;
#[allow(dead_code)]
pub(crate) mod isolate_pool;
mod loader;

pub mod registry;
pub mod thread;
pub mod vsync;

#[cfg(feature = "v8-limits")]
pub mod watchdog;

pub use registry::{
    bump_destroy_epoch, current_destroy_epoch, send_command_to_host, send_critical_command_to_host,
    shutdown_host,
};
pub use thread::spawn_host_thread;
