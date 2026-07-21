use deno_core::Extension;

pub(crate) mod image;
pub(crate) mod webgl;

pub(crate) fn rendering_extensions() -> Vec<Extension> {
    image::image_extensions()
        .into_iter()
        .chain(webgl::webgl_extensions())
        .collect()
}

pub(crate) fn rendering_lazy_extensions() -> Vec<Extension> {
    image::image_lazy_extensions()
        .into_iter()
        .chain(webgl::webgl_lazy_extensions())
        .collect()
}
