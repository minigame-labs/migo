pub type HostId = i32;

mod host;

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
    lease_surface, retire_surface, send_command_to_host, send_critical_command_to_host,
    shutdown_host,
};
pub use thread::spawn_host_thread;
