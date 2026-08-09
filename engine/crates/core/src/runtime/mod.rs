pub type HostId = i32;

mod host;
mod input_state;

pub mod registry;
mod restart_boundary;
pub mod thread;
pub mod vsync;

#[cfg(test)]
mod tests;

pub use registry::{
    HostIngress, HostIngressSendError, host_ingress, lease_surface, lease_surface_tracked,
    lease_surface_with_resource, retire_surface, send_command_to_host,
    send_critical_command_to_host, send_reliable_command_to_host, shutdown_host,
};
pub use thread::{HostThread, SpawnedSurfaceHost, spawn_host_thread, spawn_host_thread_tracked};
