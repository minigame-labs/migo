use deno_core::extension;
// TODO: move to web dir
extension!(host_v8_url,
    deps = [host_v8_console],
    esm = [
        dir "url",
        "03_url.js",
    ],
);

pub(crate) fn url_extensions() -> Vec<deno_core::Extension> {
    vec![host_v8_url::init_ops_and_esm()]
}
