use deno_core::{Extension, extension};

extension!(host_v8_update,
 esm = [
    dir "android/extensions/base/update",
    "01_update_app.js",
    "02_update_mgr.js",
 ]
);

pub fn update_extensions() -> Vec<Extension> {
    vec![host_v8_update::init_ops_and_esm()]
}
