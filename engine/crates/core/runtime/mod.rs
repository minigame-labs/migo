pub type HostId = i32;

pub(crate) mod code_cache;
mod host;
#[allow(dead_code)]
pub(crate) mod isolate_pool;
mod loader;

pub mod registry;
pub mod thread;
pub mod vsync;

#[cfg(test)]
mod tests_q12_contract;
#[cfg(test)]
mod tests_q13_contract;
#[cfg(test)]
mod tests_r4_contract;

pub use registry::{
    bump_destroy_epoch, current_destroy_epoch, send_command_to_host, send_critical_command_to_host,
    shutdown_host,
};
pub use thread::spawn_host_thread;
