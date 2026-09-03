pub type HostId = i32;

// The embedded execution: a JavaScript engine in this process, its event loop,
// and the host thread that owns both. Everything below it in this module --
// the registry, the generation boundary, the vsync clock -- is engine-neutral
// and compiled either way, because both execution modes need exactly those.
#[cfg(feature = "embedded-v8")]
mod host;
#[cfg(feature = "embedded-v8")]
mod input_state;
#[cfg(feature = "embedded-v8")]
mod session_temp;
#[cfg(feature = "embedded-v8")]
pub mod thread;

pub mod registry;
mod restart_boundary;
pub mod vsync;

#[cfg(all(test, feature = "embedded-v8"))]
mod tests;

pub use registry::{
    HostIngress, HostIngressSendError, host_ingress, lease_surface, lease_surface_tracked,
    lease_surface_with_resource, retire_surface, send_command_to_host,
    send_critical_command_to_host, send_reliable_command_to_host, shutdown_host,
};
#[cfg(feature = "embedded-v8")]
pub use thread::{HostThread, SpawnedSurfaceHost, spawn_host_thread, spawn_host_thread_tracked};
