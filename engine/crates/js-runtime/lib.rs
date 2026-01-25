use shared::op_state::HostOpState;

mod audio;
mod base;
mod console;
mod file;
mod input;
mod network;
mod rendering;
mod url;
mod utility;
mod web;

mod host_runtime;
mod js_bindings;

pub use host_runtime::HostJsRuntime;

deno_core::extension!(
    runtime,
    esm_entry_point = "ext:runtime/99_main.js",
    esm = [
        dir "",
        "98_global_scope_shared.js",
        "98_global_scope_window.js",
        "99_main.js",
    ],
);

pub fn main_extensions(host: HostOpState) -> Vec<deno_core::Extension> {
    let runtime_extensions = vec![runtime::init_ops_and_esm()];

    base::base_extensions(host)
        .into_iter()
        .chain(console::console_extensions())
        .chain(utility::utility_extensions())
        .chain(input::touch_extensions())
        .chain(file::file_extensions())
        .chain(rendering::rendering_extensions())
        .chain(web::web_extensions())
        .chain(url::url_extensions())
        .chain(network::network_extensions())
        .chain(audio::audio_extensions())
        .chain(runtime_extensions)
        .collect()
}
