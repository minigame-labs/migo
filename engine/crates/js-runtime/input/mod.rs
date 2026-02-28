use deno_core::{Extension, extension};

extension!(host_v8_touch,
esm = [
    dir "input",
    "01_touch.js",
]
);

pub fn touch_extensions() -> Vec<Extension> {
    vec![host_v8_touch::init()]
}

pub fn touch_lazy_extensions() -> Vec<Extension> {
    vec![host_v8_touch::lazy_init()]
}
