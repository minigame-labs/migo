use deno_core::{Extension, extension};

extension!(host_v8_lifecycle,
 esm = [
    dir "android/extensions/base/lifecycle",
    "01_lifecycle.js",
 ]
);

pub fn lifecycle_extensions() -> Vec<Extension> {
    vec![host_v8_lifecycle::init_ops_and_esm()]
}
