use deno_core::extension;

use crate::utility::codec::{op_decode_multi_formats, op_encode_multi_formats};

mod codec;

extension!(
    host_v8_utility,
    deps = [host_v8_console],
    ops = [op_encode_multi_formats, op_decode_multi_formats],
    esm = [
        dir "src/utility",
        "01_codec.js",
    ],
);

pub(crate) fn utility_extensions() -> Vec<deno_core::Extension> {
    vec![host_v8_utility::init()]
}

pub(crate) fn utility_lazy_extensions() -> Vec<deno_core::Extension> {
    vec![host_v8_utility::lazy_init()]
}
