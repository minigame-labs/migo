mod device;
mod env;
mod lifecycle;
mod network;
mod system;
mod ui;
mod update;
use deno_core::Extension;

pub fn base_extensions() -> Vec<Extension> {
    env::env_extensions()
        .into_iter()
        .chain(system::system_extensions())
        .chain(update::update_extensions())
        .chain(lifecycle::lifecycle_extensions())
        .chain(ui::ui_extensions())
        .chain(device::device_extensions())
        .chain(network::network_extensions())
        .collect()
}
