//! PBO (Pixel Buffer Object) Async Texture Upload
//!
//! This module provides optimized texture uploading using PBOs for OpenGL ES 3.0+.
//! PBOs allow async DMA transfers from CPU to GPU memory, reducing CPU stalls.
//!
//! Design considerations:
//! - Android API 21 compatible (OpenGL ES 3.0 required for PBO)
//! - Automatic fallback to synchronous upload on ES 2.0
//! - Memory-efficient: reuses PBO buffers when possible

use glow::HasContext;
use shared::{
    error::{EngineResult, ErrorCode},
    protocol::io_cmd::NormalizedImage,
};
use tracing::{debug, trace, warn};

use super::types::ee;

/// Configuration for PBO uploads
#[allow(dead_code)]
pub struct PboConfig {
    /// Enable PBO usage (requires ES 3.0+)
    pub enabled: bool,
    /// Maximum PBO buffer size in bytes (for reuse pool)
    pub max_buffer_size: usize,
}

impl Default for PboConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            // 4MB max buffer size - enough for a 1024x1024 RGBA image
            max_buffer_size: 4 * 1024 * 1024,
        }
    }
}

/// PBO upload result
pub struct PboUploadResult {
    pub texture: glow::NativeTexture,
    pub width: u32,
    pub height: u32,
}

/// Check if OpenGL ES 3.0+ is available (required for PBO)
///
/// Returns true if PBOs can be used
pub fn check_pbo_support(gl: &glow::Context) -> bool {
    unsafe {
        let version_str = gl.get_parameter_string(glow::VERSION);
        // OpenGL ES 3.x pattern
        if version_str.contains("OpenGL ES 3") {
            debug!("PBO support detected: {}", version_str);
            return true;
        }
        // Desktop OpenGL 3.0+ also supports PBO
        if version_str.starts_with("3.") || version_str.starts_with("4.") {
            debug!("PBO support detected (Desktop): {}", version_str);
            return true;
        }
        debug!("No PBO support: {}", version_str);
        false
    }
}

/// Upload image data to GPU texture using PBO for async transfer
///
/// This function:
/// 1. Creates a PBO and fills it with image data
/// 2. Initiates async DMA transfer from PBO to texture
/// 3. The actual transfer happens asynchronously
///
/// When a `PboPool` is provided, PBOs are reused from the pool to avoid
/// GL object churn on frequent image loads.
///
/// Note: On ES 2.0, this falls back to synchronous upload
pub fn upload_texture_with_pbo(
    gl: &glow::Context,
    image: &NormalizedImage,
    use_pbo: bool,
    pool: Option<&mut PboPool>,
) -> EngineResult<PboUploadResult> {
    let start = std::time::Instant::now();

    // Create texture
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

        // Set texture parameters
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
    }

    if use_pbo && !image.rgba.is_empty() {
        // Try pool-based PBO upload first, fall back to create-delete
        upload_with_pbo_pooled(gl, image, pool)?;
    } else {
        // Fallback - synchronous upload
        upload_sync_internal(gl, tex, image)?;
    }

    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, None);
    }

    trace!(
        "Texture uploaded: {}x{} in {:.2?} (pbo={})",
        image.width,
        image.height,
        start.elapsed(),
        use_pbo
    );

    Ok(PboUploadResult {
        texture: tex,
        width: image.width,
        height: image.height,
    })
}

/// Internal: Upload using PBO with optional pool reuse
fn upload_with_pbo_pooled(
    gl: &glow::Context,
    image: &NormalizedImage,
    mut pool: Option<&mut PboPool>,
) -> EngineResult<()> {
    let data_size = image.rgba.len();

    // Try to acquire PBO from pool, otherwise create one
    let pbo = match pool.as_mut().and_then(|p| p.acquire(gl, data_size)) {
        Some(pbo) => pbo,
        None => unsafe {
            gl.create_buffer().map_err(|e| {
                ee(
                    ErrorCode::RenderBackendError,
                    format!("create_buffer (PBO) failed: {e:?}"),
                )
            })?
        },
    };

    unsafe {
        // Bind PBO as PIXEL_UNPACK_BUFFER
        gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, Some(pbo));

        // Allocate and fill PBO with image data
        // Using STREAM_DRAW hint for upload-once data
        gl.buffer_data_u8_slice(glow::PIXEL_UNPACK_BUFFER, &image.rgba, glow::STREAM_DRAW);

        // Set pixel store alignment (RGBA is 4-byte aligned)
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);

        // Upload from PBO to texture (async DMA transfer)
        // When PBO is bound, the last parameter is interpreted as an offset into the PBO
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            image.width as i32,
            image.height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::BufferOffset(0),
        );

        // Unbind PBO
        gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
    }

    // Return PBO to pool (pool handles "full" case by deleting), or delete directly
    if let Some(p) = pool {
        p.release(gl, pbo, data_size);
    } else {
        unsafe { gl.delete_buffer(pbo) };
    }

    Ok(())
}

/// Internal: Synchronous upload (fallback for ES 2.0)
fn upload_sync_internal(
    gl: &glow::Context,
    _tex: glow::NativeTexture,
    image: &NormalizedImage,
) -> EngineResult<()> {
    unsafe {
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);

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

        Ok(())
    }
}

/// Reusable PBO pool for frequent uploads
///
/// This reduces allocation overhead for games that frequently load/unload textures
pub struct PboPool {
    /// Available PBOs sorted by size
    available: Vec<(glow::NativeBuffer, usize)>,
    /// Maximum pool size
    max_pool_size: usize,
    /// PBO support flag
    pbo_supported: bool,
}

impl PboPool {
    /// Default pool size (number of PBOs to keep).
    pub const DEFAULT_POOL_SIZE: usize = 4;

    pub fn new(gl: &glow::Context, max_pool_size: usize) -> Self {
        let pbo_supported = check_pbo_support(gl);
        Self {
            available: Vec::with_capacity(max_pool_size),
            max_pool_size: max_pool_size.max(1).min(Self::DEFAULT_POOL_SIZE * 2),
            pbo_supported,
        }
    }

    /// Check if PBOs are supported
    pub fn is_pbo_supported(&self) -> bool {
        self.pbo_supported
    }

    /// Get a PBO of at least the specified size
    pub fn acquire(&mut self, gl: &glow::Context, size: usize) -> Option<glow::NativeBuffer> {
        if !self.pbo_supported {
            return None;
        }

        // Find a suitable PBO from the pool
        if let Some(idx) = self.available.iter().position(|(_, s)| *s >= size) {
            let (pbo, _) = self.available.remove(idx);
            return Some(pbo);
        }

        // Create a new PBO
        unsafe { gl.create_buffer().ok() }
    }

    /// Return a PBO to the pool
    pub fn release(&mut self, gl: &glow::Context, pbo: glow::NativeBuffer, size: usize) {
        if self.available.len() < self.max_pool_size {
            self.available.push((pbo, size));
            // Sort by size for better allocation
            self.available.sort_by_key(|(_, s)| *s);
        } else {
            // Pool full, delete the PBO
            unsafe {
                gl.delete_buffer(pbo);
            }
        }
    }

    /// Clear the pool
    pub fn clear(&mut self, gl: &glow::Context) {
        for (pbo, _) in self.available.drain(..) {
            unsafe {
                gl.delete_buffer(pbo);
            }
        }
    }
}

impl Drop for PboPool {
    fn drop(&mut self) {
        if !self.available.is_empty() {
            warn!(
                "PboPool dropped with {} unreleased PBOs - memory leak",
                self.available.len()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require a valid GL context, so they're integration tests
    // and would be run with a test harness that provides a GL context.
}
