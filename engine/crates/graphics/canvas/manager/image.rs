//! Per-manager image registry: texture uploads + lifetime tracking.
//!
//! This is a thin wrapper around
//! [`crate::backend::gl::image_store::ImageStore`] that plugs into the
//! manager-level upload pipeline (PBO async, AHB, compressed).  It owns
//! *only* raw GL textures; `SkImage`s are constructed on demand by
//! callers that have access to a `GrDirectContext`.
//!
//! The femtovg-era per-canvas replica bookkeeping
//! (`fv_images: HashMap<image_id, HashMap<CanvasId, …>>`) has been
//! removed: Skia can wrap a shared texture into a context-specific
//! `SkImage` without copying, so per-canvas replicas were busywork.

use glow::HasContext;
use shared::{
    error::EngineResult,
    protocol::{io_cmd::NormalizedImage, render_cmd::CanvasId},
};

use crate::backend::gl::image_store::{GpuImageInfo, ImageStore, StoredImage};

use super::pbo_upload::{self, PboPool};

pub(super) struct ImageRegistry {
    store: ImageStore,
    pbo_pool: Option<PboPool>,
    use_pbo: bool,
}

impl ImageRegistry {
    pub fn new() -> Self {
        Self {
            store: ImageStore::new(),
            pbo_pool: None,
            use_pbo: true,
        }
    }

    /// Initialize PBO pool on first use (once a GL context is current).
    fn ensure_pbo_pool(&mut self, gl: &glow::Context) {
        if self.pbo_pool.is_none() {
            let pool = PboPool::new(gl, PboPool::DEFAULT_POOL_SIZE);
            self.use_pbo = pool.is_pbo_supported();
            self.pbo_pool = Some(pool);
            tracing::debug!("PBO upload initialized: enabled={}", self.use_pbo);
        }
    }

    pub fn generate_img_id(&self) -> u32 {
        self.store.generate_id()
    }

    pub fn load_shared_image(
        &mut self,
        gl: &glow::Context,
        image_id: u32,
        image: NormalizedImage,
        device_caps: &crate::device_caps::DeviceCapabilities,
        egl_display_ptr: *const std::ffi::c_void,
    ) -> EngineResult<(u32, u32)> {
        self.ensure_pbo_pool(gl);

        let result = pbo_upload::upload_texture_tiered(
            gl,
            &image,
            self.use_pbo,
            self.pbo_pool.as_mut(),
            device_caps,
            egl_display_ptr,
        )?;

        self.store.insert(
            image_id,
            StoredImage {
                gl_texture: result.texture.0.get(),
                info: GpuImageInfo::rgba8_unpremul(result.width, result.height),
            },
        );
        Ok((result.width, result.height))
    }

    /// Destroy a shared image — deletes its GL texture (if present) and
    /// removes it from the registry.
    pub fn destroy_shared_image(
        &mut self,
        gl: &glow::Context,
        image_id: u32,
    ) -> EngineResult<()> {
        if let Some(entry) = self.store.remove(image_id) {
            // SAFETY: we just removed the entry so no other live reference
            // to this texture exists in the registry.  Raw GL deletion.
            if let Some(tex) = glow::NativeTexture::try_from_raw(entry.gl_texture) {
                unsafe { gl.delete_texture(tex) };
            }
        }
        Ok(())
    }

    /// Register a pre-uploaded texture (invoked by the upload thread).
    pub fn register_shared_texture(
        &mut self,
        image_id: u32,
        texture: glow::NativeTexture,
        info: GpuImageInfo,
    ) {
        self.store.insert(
            image_id,
            StoredImage {
                gl_texture: texture.0.get(),
                info,
            },
        );
    }

    pub fn get_shared_texture(&self, image_id: u32) -> Option<StoredImage> {
        self.store.get(image_id).copied()
    }

    /// Expose the underlying store for code paths (e.g. Skia image
    /// resolution) that need the full entry plus a `GrDirectContext`.
    pub fn store(&self) -> &ImageStore {
        &self.store
    }

    /// Initialize the PBO pool (public entry for CanvasManager).
    pub fn ensure_pbo_pool_public(&mut self, gl: &glow::Context) {
        self.ensure_pbo_pool(gl);
    }

    /// Mutable access to the PBO pool for WebGL texture uploads.
    pub fn pbo_pool_mut(&mut self) -> Option<&mut PboPool> {
        self.pbo_pool.as_mut()
    }

    /// No-op: Skia-era registry keeps no per-canvas state.  Kept for
    /// source-compatibility with older call sites (which will be deleted
    /// once the transition is complete).
    pub fn remove_canvas_images(&mut self, _canvas_id: CanvasId) {}

    /// Destroy all registered textures.  Called on manager shutdown.
    pub fn destroy_all(&mut self, gl: &glow::Context) {
        // Snapshot ids first so we can mutate the store inside the loop.
        let ids: Vec<u32> = self.store.iter().map(|(k, _)| *k).collect();
        for id in ids {
            if let Some(entry) = self.store.remove(id) {
                if let Some(tex) = glow::NativeTexture::try_from_raw(entry.gl_texture) {
                    unsafe { gl.delete_texture(tex) };
                }
            }
        }
        if let Some(ref mut pool) = self.pbo_pool {
            pool.clear(gl);
        }
    }
}

/// Extension-method compatibility: glow's `NativeTexture` does not have a
/// `try_from_raw` in 0.17, so we roll one here.  The texture was created
/// by `glGenTextures`, so any non-zero `u32` round-trips safely.
trait NativeTextureFromRaw {
    fn try_from_raw(raw: u32) -> Option<glow::NativeTexture>;
}
impl NativeTextureFromRaw for glow::NativeTexture {
    #[inline]
    fn try_from_raw(raw: u32) -> Option<glow::NativeTexture> {
        std::num::NonZeroU32::new(raw).map(glow::NativeTexture)
    }
}
