//! Environment extension providing virtual path constants.
//!
//! All paths exposed to JavaScript are virtual and sandboxed.
//! The actual path resolution happens in the file system ops via VirtualFS.

use deno_core::{Extension, extension};

extension!(host_v8_env,
 esm = [
    dir "android/extensions/base/env",
    "00_env.js"
 ]
);

pub fn env_extensions() -> Vec<Extension> {
    vec![host_v8_env::init_ops_and_esm()]
}
