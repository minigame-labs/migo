//! Texture atlas auto-packing.
//!
//! Small textures (<= 256x256) are packed into 2048x2048 atlas pages to
//! reduce GL texture bind/switch overhead.
//!
//! ## Architecture
//!
//! - [`allocator`]: Pure-data shelf-based rectangle packer (no GL dependency).
//! - [`AtlasManager`]: Owns the GL atlas textures and uploads sub-images via
//!   `glTexSubImage2D`.

pub mod allocator;

pub use allocator::{AtlasAllocator, AtlasRegion, DEFAULT_ATLAS_SIZE, MAX_INPUT_DIM};

use glow::HasContext;
use tracing::{debug, warn};

/// A resolved atlas entry: GL texture handle + UV sub-region.
#[derive(Debug, Clone, Copy)]
pub struct AtlasEntry {
    /// The GL texture that contains this sub-image.
    pub texture: glow::NativeTexture,
    /// Region within the atlas (pixel coordinates).
    pub region: AtlasRegion,
}

/// Manages atlas GL textures and sub-image uploads.
///
/// All methods that touch GL **must** be called from a thread with a current
/// GL context (typically the render thread).
pub struct AtlasManager {
    alloc: AtlasAllocator,
    /// One GL texture per atlas page. Index matches `AtlasRegion::atlas_id`.
    textures: Vec<glow::NativeTexture>,
}

impl AtlasManager {
    /// Create a new manager with the default atlas size.
    pub fn new() -> Self {
        Self {
            alloc: AtlasAllocator::with_default_size(),
            textures: Vec::new(),
        }
    }

    /// Create a new manager with a custom square atlas size.
    pub fn with_atlas_size(size: u16) -> Self {
        Self {
            alloc: AtlasAllocator::new(size),
            textures: Vec::new(),
        }
    }

    /// The underlying allocator (read-only).
    pub fn allocator(&self) -> &AtlasAllocator {
        &self.alloc
    }

    /// Number of atlas pages (and GL textures) currently alive.
    pub fn page_count(&self) -> u32 {
        self.alloc.page_count()
    }

    /// Allocate space for a `w x h` RGBA sub-image and upload its pixels.
    ///
    /// Returns `None` if the image exceeds [`MAX_INPUT_DIM`] in either
    /// dimension.
    ///
    /// # Safety
    ///
    /// A GL context must be current on the calling thread.
    pub unsafe fn upload(
        &mut self,
        gl: &glow::Context,
        w: u16,
        h: u16,
        rgba: &[u8],
    ) -> Option<AtlasEntry> {
        let expected_len = (w as usize) * (h as usize) * 4;
        if rgba.len() < expected_len {
            warn!(
                "atlas upload: buffer too small (expected {expected_len}, got {})",
                rgba.len()
            );
            return None;
        }

        let region = self.alloc.allocate(w, h)?;

        // Ensure the GL texture for this page exists.
        unsafe {
            self.ensure_page_texture(gl, region.atlas_id);
        }

        let texture = self.textures[region.atlas_id as usize];

        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0, // level
                region.x as i32,
                region.y as i32,
                w as i32,
                h as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(rgba)),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
        }

        debug!(
            "atlas: uploaded {w}x{h} to page {} at ({}, {})",
            region.atlas_id, region.x, region.y,
        );

        Some(AtlasEntry { texture, region })
    }

    /// Get the GL texture handle for a given atlas page.
    pub fn page_texture(&self, atlas_id: u32) -> Option<glow::NativeTexture> {
        self.textures.get(atlas_id as usize).copied()
    }

    /// Delete all GL textures and reset the allocator.
    ///
    /// # Safety
    ///
    /// A GL context must be current on the calling thread.
    pub unsafe fn destroy(&mut self, gl: &glow::Context) {
        for tex in self.textures.drain(..) {
            unsafe {
                gl.delete_texture(tex);
            }
        }
        self.alloc.clear();
        debug!("atlas: destroyed all pages");
    }

    /// Create the GL texture for `page_id` if it does not exist yet.
    ///
    /// # Safety
    ///
    /// A GL context must be current on the calling thread.
    unsafe fn ensure_page_texture(&mut self, gl: &glow::Context, page_id: u32) {
        let idx = page_id as usize;
        if idx < self.textures.len() {
            return;
        }

        // Fill gaps (should not happen in practice).
        while self.textures.len() < idx {
            unsafe {
                if let Ok(t) = gl.create_texture() {
                    self.textures.push(t);
                }
            }
        }

        // Create the actual page texture.
        let size = self.alloc.atlas_size() as i32;
        unsafe {
            let tex = match gl.create_texture() {
                Ok(t) => t,
                Err(e) => {
                    warn!("atlas: failed to create page texture: {e}");
                    return;
                }
            };

            gl.bind_texture(glow::TEXTURE_2D, Some(tex));

            // Allocate storage with null data.
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                size,
                size,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );

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

            gl.bind_texture(glow::TEXTURE_2D, None);

            debug!("atlas: created page {page_id} texture ({size}x{size})");
            self.textures.push(tex);
        }
    }
}
