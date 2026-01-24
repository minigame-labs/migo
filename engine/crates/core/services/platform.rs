use deno_core::Extension;
use shared::config::InitOptions;

pub trait PlatformServices: Send + Sync {
    fn extensions(&self, opts: &InitOptions) -> Vec<Extension>;
}
