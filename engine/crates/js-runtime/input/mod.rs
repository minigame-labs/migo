use deno_core::{Extension, extension};

extension!(host_v8_touch,
esm = [
    dir "input",
    "01_touch.js",
]
);

pub fn touch_extensions() -> Vec<Extension> {
    vec![host_v8_touch::init_ops_and_esm()]
}
