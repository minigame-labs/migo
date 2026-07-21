use deno_core::extension;

extension!(host_v8_url,
    deps = [host_v8_console],
    esm = [
        dir "src/url",
        "03_url.js",
    ],
);

pub(crate) fn url_extensions() -> Vec<deno_core::Extension> {
    vec![host_v8_url::init()]
}

pub(crate) fn url_lazy_extensions() -> Vec<deno_core::Extension> {
    vec![host_v8_url::lazy_init()]
}
