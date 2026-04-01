use femtovg::{ImageFlags, ImageId, ImageInfo};
use glow::HasContext;
use shared::{
    error::EngineResult,
    protocol::{io_cmd::NormalizedImage, render_cmd::CanvasId},
};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU32, Ordering},
};

use super::pbo_upload::{self, PboPool};
use crate::Canvas2DContext;

/// Image registry for managing shared and per-canvas images
pub(super) struct ImageRegistry {
    /// shared femtovg images: image_id -> (texture, info)
    shared_fv_images: HashMap<u32, (glow::NativeTexture, ImageInfo)>,
    /// per-canvas owned image replicas: image_id -> canvas_id -> (fv ImageId, native_tex, info)
    pub fv_images: HashMap<u32, HashMap<CanvasId, (ImageId, glow::NativeTexture, ImageInfo)>>,
    next_image_id: AtomicU32,
    /// PBO pool for async texture uploads
    pbo_pool: Option<PboPool>,
    /// Whether PBO upload is enabled
    use_pbo: bool,
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
            pbo_pool: None, // Will be initialized on first upload
            use_pbo: true,  // Enable by default, will auto-detect support
        }
    }

    /// Initialize PBO pool (called once when GL context is available)
    fn ensure_pbo_pool(&mut self, gl: &glow::Context) {
        if self.pbo_pool.is_none() {
            let pool = PboPool::new(gl, PboPool::DEFAULT_POOL_SIZE);
            self.use_pbo = pool.is_pbo_supported();
            self.pbo_pool = Some(pool);
            tracing::debug!("PBO upload initialized: enabled={}", self.use_pbo);
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
        device_caps: &crate::device_caps::DeviceCapabilities,
        egl_display_ptr: *const std::ffi::c_void,
    ) -> EngineResult<(u32, u32)> {
        // Initialize PBO pool on first use
        self.ensure_pbo_pool(gl);

        // NOTE: Do NOT delete an existing texture for image_id here.
        // Multiple Image aliases may reference the same shared_id.
        // Deletion must only happen through the refcount=0 path in
        // IMAGE_CACHE (remove_previous_alias -> DestroyImage ->
        // destroy_shared_fv_image).

        // Use the best available upload path for this device tier:
        // TierA + API 26+: AHB → EGLImage (zero glTexImage2D).
        // TierA / TierB: PBO async upload.
        // ES 2.0: synchronous fallback.
        let result = pbo_upload::upload_texture_tiered(
            gl,
            &image,
            self.use_pbo,
            self.pbo_pool.as_mut(),
            device_caps,
            egl_display_ptr,
        )?;

        let info = ImageInfo::new(
            ImageFlags::empty(),
            result.width as usize,
            result.height as usize,
            femtovg::PixelFormat::Rgba8,
        );

        self.shared_fv_images
            .insert(image_id, (result.texture, info));
        Ok((result.width, result.height))
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

    /// Register a pre-uploaded texture (e.g. from the upload thread).
    pub fn register_shared_texture(
        &mut self,
        image_id: u32,
        texture: glow::NativeTexture,
        info: ImageInfo,
    ) {
        self.shared_fv_images.insert(image_id, (texture, info));
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

    /// Initialize PBO pool (public entry for CanvasManager).
    pub fn ensure_pbo_pool_public(&mut self, gl: &glow::Context) {
        self.ensure_pbo_pool(gl);
    }

    /// Mutable access to the PBO pool for WebGL texture uploads.
    pub fn pbo_pool_mut(&mut self) -> Option<&mut PboPool> {
        self.pbo_pool.as_mut()
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

        // Clear PBO pool
        if let Some(ref mut pool) = self.pbo_pool {
            pool.clear(gl);
        }
    }
}
