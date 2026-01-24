use core::PlatformServices;
use deno_core::Extension;
use shared::config::InitOptions;

use crate::android::extensions::android_extensions;

pub struct AndroidPlatform;

impl AndroidPlatform {
    pub fn new() -> Self {
        Self
    }
}

impl PlatformServices for AndroidPlatform {
    fn extensions(&self, opts: &InitOptions) -> Vec<Extension> {
        android_extensions(opts)
    }
}
