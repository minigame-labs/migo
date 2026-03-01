//! App update extension for update checking and applying.

use deno_core::{Extension, extension};

extension!(host_v8_update,
    esm = [
        dir "update",
        "01_update_app.js",
        "02_update_mgr.js",
    ]
);

pub fn update_extensions() -> Vec<Extension> {
    vec![host_v8_update::init()]
}
