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
///
/// `sk_image_cache` memoises the `backend_textures::make_gl` +
/// `borrow_texture_from` pair.  Each produced `SkImage` is bound to a
/// specific `DirectContext`, so the key is `(image_id,
/// DirectContextId.id())`; a second canvas (different DirectContext)
/// will lazily populate its own entry on first draw.  `SkImage` is
/// an `RCHandle`, so cloning it on cache hit is a refcount bump.
#[derive(Default)]
pub struct ImageStore {
    entries: HashMap<u32, StoredImage>,
    next_id: AtomicU32,
    sk_image_cache: HashMap<(u32, u32), SkImage>,
}

impl ImageStore {
    pub fn new() -> Self {
        Self {
            entries: HashMap::with_capacity(32),
            next_id: AtomicU32::new(1),
            sk_image_cache: HashMap::with_capacity(32),
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
        // Evict every SkImage wrapper that pointed at this image id
        // so a future insert with the same id doesn't accidentally
        // resolve to a stale backend texture.  HashMap iteration
        // requires a Vec detour because we're mutating in place.
        let stale_keys: Vec<(u32, u32)> = self
            .sk_image_cache
            .keys()
            .filter(|(id, _)| *id == image_id)
            .copied()
            .collect();
        for k in stale_keys {
            self.sk_image_cache.remove(&k);
        }
        self.entries.remove(&image_id)
    }

    pub fn get(&self, image_id: u32) -> Option<&StoredImage> {
        self.entries.get(&image_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u32, &StoredImage)> {
        self.entries.iter()
    }

    /// Wrap an existing GL texture into an `SkImage` owned by `gr_ctx`,
    /// bypassing the wrapper cache.  Kept for diagnostic use and tests;
    /// production code should use [`Self::resolve_cached_or_wrap`] so
    /// repeated `drawImage` of the same texture reuses the `SkImage`
    /// handle and skips the `backend_textures::make_gl` +
    /// `borrow_texture_from` pair.
    ///
    /// The returned image borrows the texture -- it does NOT own it, and
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

    /// Fetch the `SkImage` wrapper for `image_id` against `gr_ctx`,
    /// building + caching it on first use.  Repeated calls at the
    /// same `(image_id, gr_ctx)` pair hand back a refcounted clone
    /// of the prior wrapper without touching the Ganesh backend
    /// texture machinery -- the performance-critical case for games
    /// that call `drawImage(sameSprite, ...)` hundreds of times per
    /// frame.
    ///
    /// Returns `None` when the `image_id` isn't in the store or the
    /// Ganesh wrap fails (OOM / driver issue).  Cache entries are
    /// scoped by `DirectContext::id()` so two Canvas2DContexts can
    /// independently cache their own wrappers without collisions.
    pub fn resolve_cached_or_wrap(
        &mut self,
        ctx_tag: u32,
        gr_ctx: &mut gpu::DirectContext,
        image_id: u32,
    ) -> Option<SkImage> {
        let key = (image_id, ctx_tag);
        if let Some(hit) = self.sk_image_cache.get(&key) {
            crate::render_diagnostics::hit_sk_image_wrapper();
            return Some(hit.clone());
        }
        crate::render_diagnostics::miss_sk_image_wrapper();
        let entry = self.entries.get(&image_id)?.clone();
        let wrapped = Self::resolve_as_sk_image(gr_ctx, &entry)?;
        self.sk_image_cache.insert(key, wrapped.clone());
        Some(wrapped)
    }

    /// Drop every SkImage wrapper whose DirectContext has been torn
    /// down.  Called from `Canvas2DContext::Drop` / manager shutdown
    /// so stale wrappers don't dangle past their backing GrContext.
    #[allow(dead_code)]
    pub fn purge_wrappers_for_context(&mut self, ctx_id: u32) {
        self.sk_image_cache.retain(|(_, c), _| *c != ctx_id);
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
    fn remove_evicts_sk_image_cache_entries_for_that_id() {
        // Regression: re-inserting the same image id with a fresh
        // backing GL texture must NOT serve a stale SkImage wrapper
        // from before the first remove().  Without the eviction in
        // `ImageStore::remove`, a `drawImage` after the re-insert
        // would render the old texture content.
        let mut s = ImageStore::new();
        let id = s.generate_id();
        s.insert(
            id,
            StoredImage {
                gl_texture: 1,
                info: GpuImageInfo::rgba8_unpremul(4, 4),
            },
        );
        // Populate the cache directly (can't call resolve_cached_or_wrap
        // without a live DirectContext in pure-Rust tests).
        // The key shape is what ::remove invalidates by.
        s.sk_image_cache.insert(
            (id, 0xAAAA_AAAA),
            // Invalid placeholder SkImage - this test only checks
            // the map eviction semantics, never touches the image.
            // We drop the entry via `remove`; the placeholder is
            // immediately forgotten.
            {
                // A trivial 1x1 raster SkImage works as a placeholder.
                let info = skia_safe::ImageInfo::new(
                    (1, 1),
                    ColorType::RGBA8888,
                    AlphaType::Premul,
                    None,
                );
                let mut surf = skia_safe::surfaces::raster(&info, None, None).unwrap();
                surf.image_snapshot()
            },
        );
        assert_eq!(s.sk_image_cache.len(), 1);
        let _ = s.remove(id);
        assert_eq!(s.sk_image_cache.len(), 0, "remove must drop wrappers");
    }

    #[test]
    fn purge_wrappers_for_context_is_scoped() {
        // Build a two-context shape: ctx_tag A has two images, ctx B
        // has one.  Purging A's wrappers leaves B's untouched.
        let mut s = ImageStore::new();
        let info = skia_safe::ImageInfo::new(
            (1, 1),
            ColorType::RGBA8888,
            AlphaType::Premul,
            None,
        );
        let mut mk_img = || {
            skia_safe::surfaces::raster(&info, None, None)
                .unwrap()
                .image_snapshot()
        };
        s.sk_image_cache.insert((1, 100), mk_img());
        s.sk_image_cache.insert((2, 100), mk_img());
        s.sk_image_cache.insert((3, 200), mk_img());
        s.purge_wrappers_for_context(100);
        assert_eq!(s.sk_image_cache.len(), 1);
        assert!(s.sk_image_cache.contains_key(&(3, 200)));
    }

    /// Simulate the `Canvas2DContext::resize` flow: a wrapper keyed
    /// on `(image_id, ctx_tag_old)` must no longer match lookups
    /// after we rotate `ctx_tag_old → ctx_tag_new` and purge the
    /// old tag.  Without the purge, a bug in the `(image_id, tag)`
    /// match would silently serve a wrapper bound to a destroyed
    /// `GrDirectContext`; this test locks the invariant.
    #[test]
    fn purge_then_new_ctx_tag_isolates_pre_resize_wrappers() {
        let mut s = ImageStore::new();
        let info = skia_safe::ImageInfo::new(
            (1, 1),
            ColorType::RGBA8888,
            AlphaType::Premul,
            None,
        );
        let mut mk_img = || {
            skia_safe::surfaces::raster(&info, None, None)
                .unwrap()
                .image_snapshot()
        };
        // Pre-resize: ctx_tag = 42 uses image_id = 7.
        s.sk_image_cache.insert((7, 42), mk_img());
        assert!(s.sk_image_cache.contains_key(&(7, 42)));

        // Resize flow: purge old tag, then install new tag's wrapper.
        s.purge_wrappers_for_context(42);
        assert!(s.sk_image_cache.is_empty());

        s.sk_image_cache.insert((7, 43), mk_img());
        // Old key is gone; new key is reachable.  A re-resize
        // (42 → 43 → 44) can continue the pattern without ever
        // colliding with stale wrappers.
        assert!(!s.sk_image_cache.contains_key(&(7, 42)));
        assert!(s.sk_image_cache.contains_key(&(7, 43)));
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
