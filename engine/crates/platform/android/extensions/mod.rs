use shared::config::InitOptions;

use crate::android::extensions::base::base_extensions;

mod base;
mod file;

deno_core::extension!(
    android_runtime,
    esm_entry_point = "ext:android_runtime/android_main.js",
    esm = [
        dir "android/extensions",
        "android_main.js",
    ],
);

pub fn android_extensions(_: &InitOptions) -> Vec<deno_core::Extension> {
    let runtime_extensions = vec![android_runtime::init_ops_and_esm()];

    base_extensions()
        .into_iter()
        .chain(file::file_extensions())
        .chain(runtime_extensions)
        .collect()
}
