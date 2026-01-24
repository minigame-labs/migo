use deno_core::Extension;

pub(crate) mod image;
pub(crate) mod webgl;

pub(crate) fn rendering_extensions() -> Vec<Extension> {
    image::image_extensions()
        .into_iter()
        .chain(webgl::webgl_extensions())
        .collect()
}
