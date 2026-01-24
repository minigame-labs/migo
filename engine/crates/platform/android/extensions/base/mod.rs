mod env;
mod lifecycle;
mod system;
mod update;
use deno_core::Extension;

pub fn base_extensions() -> Vec<Extension> {
    env::env_extensions()
        .into_iter()
        .chain(system::system_extensions())
        .chain(update::update_extensions())
        .chain(lifecycle::lifecycle_extensions())
        .collect()
}
