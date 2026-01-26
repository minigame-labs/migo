use femtovg::{ImageFlags, ImageId, ImageInfo};
use glow::HasContext;
use shared::{
    error::{EngineResult, ErrorCode},
    protocol::{io_cmd::NormalizedImage, render_cmd::CanvasId},
};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU32, Ordering},
};

use super::types::ee;
use crate::Canvas2DContext;

/// Image registry for managing shared and per-canvas images
pub(super) struct ImageRegistry {
    /// shared femtovg images: image_id -> (texture, info)
    shared_fv_images: HashMap<u32, (glow::NativeTexture, ImageInfo)>,
    /// per-canvas owned image replicas: image_id -> canvas_id -> (fv ImageId, native_tex, info)
    pub fv_images: HashMap<u32, HashMap<CanvasId, (ImageId, glow::NativeTexture, ImageInfo)>>,
    next_image_id: AtomicU32,
}

impl ImageRegistry {
    /// Default capacity for image maps.
    /// Most games load a moderate number of images.
    const DEFAULT_IMAGE_CAPACITY: usize = 32;

    pub fn new() -> Self {
        Self {
            shared_fv_images: HashMap::with_capacity(Self::DEFAULT_IMAGE_CAPACITY),
            fv_images: HashMap::with_capacity(Self::DEFAULT_IMAGE_CAPACITY),
            next_image_id: AtomicU32::new(1),
        }
    }

    pub fn generate_img_id(&self) -> u32 {
        self.next_image_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn load_shared_fv_image(
        &mut self,
        gl: &glow::Context,
        image_id: u32,
        image: NormalizedImage,
    ) -> EngineResult<(u32, u32)> {
        let tex = unsafe {
            gl.create_texture().map_err(|e| {
                ee(
                    ErrorCode::RenderBackendError,
                    format!("create_texture failed: {e:?}"),
                )
            })?
        };

        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));

            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);

            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );

            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );

            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                image.width as i32,
                image.height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&image.rgba)),
            );

            gl.bind_texture(glow::TEXTURE_2D, None);
        }

        let info = ImageInfo::new(
            ImageFlags::empty(),
            image.width as usize,
            image.height as usize,
            femtovg::PixelFormat::Rgba8,
        );

        self.shared_fv_images.insert(image_id, (tex, info));
        Ok((image.width, image.height))
    }

    pub fn destroy_shared_fv_image<F>(
        &mut self,
        gl: &glow::Context,
        image_id: u32,
        mut make_current: F,
        contexts_2d: &mut HashMap<CanvasId, Canvas2DContext>,
    ) -> EngineResult<()>
    where
        F: FnMut(CanvasId) -> EngineResult<()>,
    {
        if let Some(per_canvas) = self.fv_images.remove(&image_id) {
            for (canvas_id, (fv_id, _native_tex, _info)) in per_canvas {
                make_current(canvas_id)?;

                if let Some(ctx2d) = contexts_2d.get_mut(&canvas_id) {
                    ctx2d.canvas.delete_image(fv_id)
                }
            }
        }

        if let Some((tex, _info)) = self.shared_fv_images.remove(&image_id) {
            unsafe { gl.delete_texture(tex) };
        }

        Ok(())
    }

    pub fn get_shared_fv_image(
        &self,
        image_id: u32,
    ) -> Option<(u32, glow::NativeTexture, ImageInfo)> {
        self.shared_fv_images
            .get(&image_id)
            .map(|(t, info)| (image_id, *t, *info))
    }

    pub fn get_owned_fv_image(
        &self,
        image_id: u32,
        canvas_id: CanvasId,
    ) -> Option<(ImageId, glow::NativeTexture, ImageInfo)> {
        self.fv_images
            .get(&image_id)
            .and_then(|m| m.get(&canvas_id))
            .cloned()
    }

    /// Remove all per-canvas images for a specific canvas
    pub fn remove_canvas_images(&mut self, canvas_id: CanvasId) {
        for (_img_id, per) in self.fv_images.iter_mut() {
            per.remove(&canvas_id);
        }
    }

    /// Cleanup all images
    pub fn destroy_all(&mut self, gl: &glow::Context) {
        for (_id, (tex, _info)) in self.shared_fv_images.drain() {
            unsafe { gl.delete_texture(tex) };
        }
        self.fv_images.clear();
    }
}
