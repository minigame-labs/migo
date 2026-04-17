//! Skia-friendly image registry.
//!
//! Replaces the femtovg `ImageId` bookkeeping with a thin
//! `image_id → (GL texture, dimensions)` map.  Pattern / drawImage paths
//! wrap a texture into an `SkImage` *on demand* via
//! [`resolve_as_sk_image`], because an `SkImage` is tied to a specific
//! [`DirectContext`] and canvases don't share one.
//!
//! Off-screen canvases that use the same texture would each create their
//! own transient `SkImage` on first reference.  This is cheap — Skia's
//! [`images::borrow_from_backend_texture`] does no upload, just a
//! metadata wrap.
//!
//! [`images::borrow_from_backend_texture`]: https://skia.org/docs/user/api/gpu/

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use skia_safe::{
    gpu::{
        self,
        backend_textures,
        gl::{self as sk_gl, TextureInfo},
        Mipmapped, SurfaceOrigin,
    },
    AlphaType, ColorType, Image as SkImage,
};

/// Metadata we retain for every uploaded image.  Kept deliberately minimal;
/// everything else (shaders, sub-images, …) is rebuilt on demand.
#[derive(Clone, Copy, Debug)]
pub struct GpuImageInfo {
    pub width: u32,
    pub height: u32,
    /// Colour type used when wrapping the texture into an `SkImage`.
    /// Separated from `has_alpha` to accommodate future premultiplied-BGRA
    /// uploads (e.g. AHB external textures on Mali).
    pub color_type: ColorType,
    pub alpha_type: AlphaType,
}

impl GpuImageInfo {
    pub fn rgba8_unpremul(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            color_type: ColorType::RGBA8888,
            alpha_type: AlphaType::Unpremul,
        }
    }
}

/// One entry in the image store.  `texture` is a raw GLuint held by the
/// engine (uploaded via PBO or AHB from the `upload_thread`).  Ownership
/// is exclusive: when the refcount hits zero (driven by the JS-side
/// cache) we call `glDeleteTextures` and drop the entry.
#[derive(Clone, Copy, Debug)]
pub struct StoredImage {
    /// Raw GL texture name (GLuint).
    pub gl_texture: u32,
    pub info: GpuImageInfo,
}

/// Shared image registry.  Lives on the render thread — not `Send` because
/// `glow::Context` is `!Send` on most platforms and adjacent bookkeeping
/// would cross that boundary.
#[derive(Default)]
pub struct ImageStore {
    entries: HashMap<u32, StoredImage>,
    next_id: AtomicU32,
}

impl ImageStore {
    pub fn new() -> Self {
        Self {
            entries: HashMap::with_capacity(32),
            next_id: AtomicU32::new(1),
        }
    }

    /// Reserve a fresh image id.  Always `> 0`.
    pub fn generate_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn insert(&mut self, image_id: u32, entry: StoredImage) {
        self.entries.insert(image_id, entry);
    }

    pub fn remove(&mut self, image_id: u32) -> Option<StoredImage> {
        self.entries.remove(&image_id)
    }

    pub fn get(&self, image_id: u32) -> Option<&StoredImage> {
        self.entries.get(&image_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u32, &StoredImage)> {
        self.entries.iter()
    }

    /// Wrap an existing GL texture into an `SkImage` owned by `gr_ctx`.
    ///
    /// The returned image borrows the texture — it does NOT own it, and
    /// the caller must keep the underlying GL texture alive for the
    /// lifetime of the image (release on `DestroyImage`).
    pub fn resolve_as_sk_image(
        gr_ctx: &mut gpu::DirectContext,
        entry: &StoredImage,
    ) -> Option<SkImage> {
        let tex_info = TextureInfo {
            target: gles_texture_2d(),
            id: entry.gl_texture,
            format: gl_rgba8_size(),
            protected: gpu::Protected::No,
        };
        // SAFETY: `gl_texture` is a live GLuint owned by the render thread
        // (populated via `pbo_upload` or AHB on a shared GL context).  The
        // caller guarantees (via DestroyImage reference-counting on the
        // JS side) that the texture outlives any `SkImage` we hand out.
        let backend_tex = unsafe {
            backend_textures::make_gl(
                (entry.info.width as i32, entry.info.height as i32),
                Mipmapped::No,
                tex_info,
                "migo.image_store",
            )
        };
        gpu::images::borrow_texture_from(
            gr_ctx,
            &backend_tex,
            SurfaceOrigin::TopLeft,
            entry.info.color_type,
            entry.info.alpha_type,
            None,
        )
    }
}

/// GLES2 / GLES3 enum value for `GL_TEXTURE_2D`.
#[inline]
fn gles_texture_2d() -> u32 {
    0x0DE1
}

/// GLES3 sized internal format `GL_RGBA8` (`0x8058`).  Matches the
/// Chromium-style RGBA8 attachments we create in `drawing_buffer.rs`.
#[inline]
fn gl_rgba8_size() -> u32 {
    0x8058
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_id_is_monotonic_nonzero() {
        let s = ImageStore::new();
        let a = s.generate_id();
        let b = s.generate_id();
        assert_ne!(a, 0);
        assert!(b > a);
    }

    #[test]
    fn insert_then_get_roundtrip() {
        let mut s = ImageStore::new();
        let id = s.generate_id();
        s.insert(
            id,
            StoredImage {
                gl_texture: 42,
                info: GpuImageInfo::rgba8_unpremul(64, 64),
            },
        );
        let got = s.get(id).expect("missing");
        assert_eq!(got.gl_texture, 42);
        assert_eq!(got.info.width, 64);
    }

    #[test]
    fn remove_returns_entry_once() {
        let mut s = ImageStore::new();
        let id = s.generate_id();
        s.insert(
            id,
            StoredImage {
                gl_texture: 1,
                info: GpuImageInfo::rgba8_unpremul(1, 1),
            },
        );
        assert!(s.remove(id).is_some());
        assert!(s.remove(id).is_none());
        assert!(s.get(id).is_none());
    }

    #[test]
    fn rgba8_unpremul_factory_has_expected_fields() {
        let info = GpuImageInfo::rgba8_unpremul(10, 20);
        assert_eq!(info.width, 10);
        assert_eq!(info.height, 20);
        assert_eq!(info.color_type, ColorType::RGBA8888);
        assert_eq!(info.alpha_type, AlphaType::Unpremul);
    }
}
