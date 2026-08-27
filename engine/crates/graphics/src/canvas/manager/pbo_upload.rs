//! PBO (Pixel Buffer Object) Async Texture Upload
//!
//! This module provides optimized texture uploading using PBOs for OpenGL ES 3.0+.
//! PBOs allow async DMA transfers from CPU to GPU memory, reducing CPU stalls.
//!
//! Design considerations:
//! - Requires OpenGL ES 3.0 (PBO + `glFenceSync`); the engine's minSdk is
//!   API 26, which guarantees ES 3.0 on conforming devices. The PBO/fence
//!   entry points themselves are ES 3.0, not API 21 — an earlier version of
//!   this note said "API 21 compatible", which was misleading.
//! - Automatic fallback to synchronous upload on ES 2.0 / TierB drivers
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

#[inline]
fn should_try_ahb_upload(
    device_available: bool,
    session_enabled: bool,
    display_available: bool,
) -> bool {
    device_available && session_enabled && display_available
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

/// Extended version that additionally knows whether the driver supports
/// `glTexStorage2D`.  When `true`, we allocate the texture immutably up
/// front (one driver-side layout decision instead of per-level guessing)
/// and use `glTexSubImage2D` to populate; measurably faster on Mali /
/// Adreno / PowerVR in micro-benchmarks (~10-25% on first-frame upload
/// cost for mid-sized sprite atlases).  Falls back to the traditional
/// `glTexImage2D` path on allocation failure.
pub fn upload_texture_with_pbo_ext(
    gl: &glow::Context,
    image: &NormalizedImage,
    use_pbo: bool,
    pool: Option<&mut PboPool>,
    has_tex_storage: bool,
) -> EngineResult<PboUploadResult> {
    let start = std::time::Instant::now();

    // What the caller's context had bound, so it can have it back. On the upload
    // thread nobody is watching; on the render thread the WebGL dedup shadow still
    // names the content's texture and alignment, and binding *zero* here instead of
    // restoring made the content's next identical `bindTexture` / `pixelStorei`
    // look redundant — dropped, leaving the upload or sample after it pointed at no
    // texture. `compressed_upload.rs` already restores for exactly this reason;
    // these two upload paths were the ones that did not.
    let saved = unsafe {
        (
            gl.get_parameter_i32(glow::TEXTURE_BINDING_2D),
            gl.get_parameter_i32(glow::UNPACK_ALIGNMENT),
        )
    };

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

    // Choose the best upload path.  See `TextureUploadPath` for the decision
    // table; this is a one-line wrapper to keep the business logic testable.
    let path = TextureUploadPath::select(use_pbo, has_tex_storage, !image.rgba.is_empty());
    match path {
        TextureUploadPath::PboImmutable => {
            upload_immutable_with_pbo(gl, image, pool)?;
        }
        TextureUploadPath::PboMutable => {
            upload_with_pbo_pooled(gl, image, pool)?;
        }
        TextureUploadPath::Synchronous => {
            upload_sync_internal(gl, tex, image)?;
        }
    }

    unsafe {
        let (saved_texture, saved_alignment) = saved;
        gl.bind_texture(
            glow::TEXTURE_2D,
            std::num::NonZeroU32::new(saved_texture as u32).map(glow::NativeTexture),
        );
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, saved_alignment);
    }

    trace!(
        "Texture uploaded: {}x{} in {:.2?} (path={:?})",
        image.width,
        image.height,
        start.elapsed(),
        path,
    );

    Ok(PboUploadResult {
        texture: tex,
        width: image.width,
        height: image.height,
    })
}

/// Classification of which texture-upload path `upload_texture_with_pbo_ext`
/// will choose.  Factored out so the decision logic has unit tests
/// independent of any GL driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureUploadPath {
    /// `glTexStorage2D` + `glTexSubImage2D` via a PBO.  Fastest.
    PboImmutable,
    /// `glTexImage2D` via a PBO.  Async DMA transfer, driver chooses layout.
    PboMutable,
    /// `glTexImage2D` with the pixel slice directly.  Synchronous, fallback.
    Synchronous,
}

impl TextureUploadPath {
    /// Pure-function decision table.
    ///
    /// * `has_data = false` forces the synchronous path because `glTexStorage2D`
    ///   + `glTexSubImage2D(NULL)` leaves the texture uninitialised, which
    ///   would create a visible "black frame" for games that allocate then
    ///   uploading later.  The synchronous path with a zero-length slice
    ///   at least lets the driver skip the copy.
    pub fn select(use_pbo: bool, has_tex_storage: bool, has_data: bool) -> Self {
        if !has_data {
            return Self::Synchronous;
        }
        if use_pbo && has_tex_storage {
            return Self::PboImmutable;
        }
        if use_pbo {
            return Self::PboMutable;
        }
        Self::Synchronous
    }
}

/// Internal: Upload through an immutable `glTexStorage2D` allocation +
/// `glTexSubImage2D` filled via a PBO.  Fastest path on GLES 3.0+.
///
/// Caller must have already `glBindTexture`-d the destination.
fn upload_immutable_with_pbo(
    gl: &glow::Context,
    image: &NormalizedImage,
    mut pool: Option<&mut PboPool>,
) -> EngineResult<()> {
    let data_size = image.rgba.len();

    // Allocate the texture storage immutably (one big driver-side
    // decision on layout, tiling, compression …).
    // `GL_RGBA8` is 0x8058 (matches backend::gl::surface).
    unsafe {
        gl.tex_storage_2d(
            glow::TEXTURE_2D,
            /* levels */ 1,
            /* internal_format */ 0x8058,
            image.width as i32,
            image.height as i32,
        );
    }

    // Acquire or create a PBO, fill it, and copy into the texture via
    // TexSubImage2D (equivalent to the classic path but against an
    // already-allocated texture).
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
        gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, Some(pbo));
        gl.buffer_data_u8_slice(glow::PIXEL_UNPACK_BUFFER, &image.rgba, glow::STREAM_DRAW);
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
        gl.tex_sub_image_2d(
            glow::TEXTURE_2D,
            /* level */ 0,
            /* xoffset */ 0,
            /* yoffset */ 0,
            image.width as i32,
            image.height as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::BufferOffset(0),
        );
        gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
    }

    if let Some(p) = pool {
        p.release(gl, pbo, data_size);
    } else {
        unsafe { gl.delete_buffer(pbo) };
    }

    Ok(())
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

/// Reusable PBO pool for frequent uploads.
///
/// This reduces allocation overhead for games that frequently load/unload textures.
/// Each returned PBO carries a fence sync so that on Adreno GPUs (and others)
/// we do not re-use a PBO whose previous DMA transfer has not completed.
pub struct PboPool {
    /// Available PBOs: (buffer, capacity, optional fence from last upload).
    available: Vec<(glow::NativeBuffer, usize, Option<glow::NativeFence>)>,
    /// Maximum pool size
    max_pool_size: usize,
    /// PBO support flag
    pbo_supported: bool,
    /// Whether GL fence sync is available (ES 3.0+, same as PBO).
    fence_supported: bool,
}

impl PboPool {
    /// Default pool size (number of PBOs to keep).
    pub const DEFAULT_POOL_SIZE: usize = 4;

    pub fn new(gl: &glow::Context, max_pool_size: usize) -> Self {
        let pbo_supported = check_pbo_support(gl);
        // Fence sync requires the same ES 3.0 context as PBOs.
        let fence_supported = pbo_supported;
        Self {
            available: Vec::with_capacity(max_pool_size),
            max_pool_size: max_pool_size.max(1).min(Self::DEFAULT_POOL_SIZE * 2),
            pbo_supported,
            fence_supported,
        }
    }

    /// Check if PBOs are supported
    pub fn is_pbo_supported(&self) -> bool {
        self.pbo_supported
    }

    /// Get a PBO of at least the specified size.
    ///
    /// If the PBO carries a fence from a previous upload, waits (with a short
    /// timeout) for the DMA transfer to complete before returning it.  This
    /// prevents GPU stalls on Adreno and similar drivers where reusing a PBO
    /// whose transfer is still in flight causes the driver to block.
    ///
    /// # Open question: this wait may be unnecessary, and it is expensive
    ///
    /// **Every write to a PBO from this pool is a full respecify.** All four
    /// sites — [`upload_pbo_mutable`], [`upload_pbo_immutable`], and the two
    /// WebGL paths in `renderergl::handler` — call
    /// `buffer_data_u8_slice(PIXEL_UNPACK_BUFFER, .., STREAM_DRAW)` over the
    /// whole buffer; none uses `buffer_sub_data` or a mapping. A conforming
    /// GLES 3.0 driver orphans the previous storage on that call (§2.9), which
    /// is exactly why `STREAM_DRAW` plus a full `glBufferData` is the canonical
    /// stall-free streaming idiom — and makes the hazard this waits for
    /// disappear one call later.
    ///
    /// This engine's *other* PBO pool says so, in
    /// [`crate::upload_thread`]: "`STREAM_DRAW` with a full-buffer replacement
    /// orphans the driver-side storage (per spec §6.2), so reuse is safe even
    /// though the previous DMA may not have completed on the GPU." That pool
    /// keeps no fences and waits for nothing.
    ///
    /// The cost of being wrong about it is not small. The timeout is 5 ms
    /// against a 16.67 ms frame, this runs on the render thread, and a wait
    /// that *does* expire also deletes the buffer and creates a fresh one —
    /// so a sustained GPU overrun turns the pool into a create/delete treadmill
    /// on the exact frames that could least afford it.
    ///
    /// **Not removed, because the comment above is a coherent claim about
    /// non-conforming hardware and not a misreading.** It asserts that Adreno
    /// blocks *inside* `glBufferData` rather than orphaning, in which case this
    /// converts an unbounded driver stall into a bounded one plus a fallback.
    /// Settling it needs one measurement on an Adreno device: time
    /// `glBufferData` on a PBO with an unsignalled fence, with and without the
    /// wait. If the driver does orphan, the better fix is not to delete this
    /// but to make the orphan explicit — `glBufferData(target, size, NULL)`
    /// before the data upload — which costs nothing and needs no fence at all.
    ///
    /// Whoever runs that measurement should also reconcile the two pools: they
    /// currently implement opposite policies for the same access pattern, and
    /// only one of them can be right.
    pub fn acquire(&mut self, gl: &glow::Context, size: usize) -> Option<glow::NativeBuffer> {
        if !self.pbo_supported {
            return None;
        }

        // Find a suitable PBO from the pool.
        if let Some(idx) = self.available.iter().position(|(_, s, _)| *s >= size) {
            let (pbo, _, fence) = self.available.remove(idx);
            // Wait for previous DMA to finish before reusing the buffer.
            if let Some(f) = fence {
                let status = unsafe {
                    // 5 ms timeout — generous enough for any reasonable DMA.
                    gl.client_wait_sync(f, glow::SYNC_FLUSH_COMMANDS_BIT, 5_000_000)
                };
                unsafe { gl.delete_sync(f) };
                if status == glow::TIMEOUT_EXPIRED || status == glow::WAIT_FAILED {
                    // DMA still in flight or driver error — discard this PBO
                    // and create a fresh one to avoid a GPU stall.
                    warn!(
                        "PBO fence wait failed (status=0x{:X}), discarding PBO",
                        status
                    );
                    unsafe { gl.delete_buffer(pbo) };
                    return unsafe { gl.create_buffer().ok() };
                }
            }
            return Some(pbo);
        }

        // Create a new PBO.
        unsafe { gl.create_buffer().ok() }
    }

    /// Return a PBO to the pool, inserting a fence so the next `acquire`
    /// can wait for the in-flight DMA to complete.
    pub fn release(&mut self, gl: &glow::Context, pbo: glow::NativeBuffer, size: usize) {
        if self.available.len() < self.max_pool_size {
            let fence = if self.fence_supported {
                unsafe { gl.fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0).ok() }
            } else {
                None
            };
            self.available.push((pbo, size, fence));
            // Sort by size for better allocation.
            self.available.sort_by_key(|(_, s, _)| *s);
        } else {
            // Pool full, delete the PBO.
            unsafe {
                gl.delete_buffer(pbo);
            }
        }
    }

    /// Clear the pool
    pub fn clear(&mut self, gl: &glow::Context) {
        for (pbo, _, fence) in self.available.drain(..) {
            unsafe {
                if let Some(f) = fence {
                    gl.delete_sync(f);
                }
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

// ---------------------------------------------------------------------------
// AHardwareBuffer import path (API 26+, TierA)
// ---------------------------------------------------------------------------

/// Upload a decoded image using the best available path for the device tier.
///
/// - **TierA + API 26+**: AHardwareBuffer → EGLImage → GL texture (zero `glTexImage2D`).
/// - **TierA / TierB**: PBO async upload (current path).
/// - **ES 2.0 fallback**: synchronous `glTexImage2D`.
///
/// `egl_display_ptr` is the raw `EGLDisplay` pointer, needed only for AHB import.
/// Pass `std::ptr::null()` if AHB is not available.
pub fn upload_texture_tiered(
    gl: &glow::Context,
    image: &NormalizedImage,
    use_pbo: bool,
    pool: Option<&mut PboPool>,
    device_caps: &crate::device_caps::DeviceCapabilities,
    gpu_caps: &shared::device::gpu_caps::GpuCaps,
    egl_display_ptr: *const std::ffi::c_void,
) -> EngineResult<PboUploadResult> {
    let try_ahb = should_try_ahb_upload(
        device_caps.ahb_available,
        gpu_caps.snapshot().ahb,
        !egl_display_ptr.is_null(),
    );

    // AHB path: decode RGBA → AHardwareBuffer → EGLImage → GL texture.
    #[cfg(target_os = "android")]
    if try_ahb {
        match try_ahb_upload(gl, image, egl_display_ptr) {
            Ok(result) => return Ok(result),
            Err(e) => {
                if gpu_caps.disable_ahb() {
                    shared::stats::io_metrics_global().record_ahb_fallback(
                        shared::stats::AhbFallbackReason::HardwareBufferUnavailable,
                    );
                    shared::warn_once!(
                        "Legacy RGBA-to-AHB upload failed; disabling AHB for this host session ({e})"
                    );
                }
                debug!("AHB upload failed, falling back to PBO: {e}");
            }
        }
    }

    #[cfg(not(target_os = "android"))]
    let _ = try_ahb; // selection is still unit-tested off-Android

    // Fallback: immutable PBO, mutable PBO, or synchronous upload.
    let has_tex_storage = device_caps.gles_version >= (3, 0);
    upload_texture_with_pbo_ext(gl, image, use_pbo, pool, has_tex_storage)
}

#[cfg(target_os = "android")]
fn try_ahb_upload(
    gl: &glow::Context,
    image: &NormalizedImage,
    egl_display_ptr: *const std::ffi::c_void,
) -> Result<PboUploadResult, String> {
    use shared::protocol::ahb::{AhbDesc, OwnedAhb, write_rgba_into_ahb};

    // Legacy RGBA path: allocate an AHB here and memcpy the RGBA
    // payload in. Kept so `upload_texture_tiered` still has a useful
    // AHB fallback for non-Android decoders or cached-RGBA hits.  The
    // zero-memcpy path — decoder writes straight into AHB — lives in
    // [`upload_ahb_image`]; `CanvasCmd::LoadImage` routes to it when
    // the caller has a [`DecodedImage::HardwareBuffer`] already.
    let ahb = OwnedAhb::allocate(AhbDesc::rgba_sampled_cpu_decode(image.width, image.height))
        .map_err(|e| format!("AHB allocate: {e}"))?;
    write_rgba_into_ahb(&ahb, &image.rgba).map_err(|e| format!("AHB write: {e}"))?;

    let result = unsafe {
        crate::texture_import::import_ahb_as_texture(
            gl,
            ahb.raw(),
            egl_display_ptr,
            image.width,
            image.height,
        )
    }
    .map_err(|e| format!("{e}"))?;

    Ok(PboUploadResult {
        texture: result.texture,
        width: result.width,
        height: result.height,
    })
    // ahb dropped here → AHardwareBuffer_release (EGLImage holds its own ref)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Path selection is a pure function — exhaustive decision-table tests
    // below run without any GL context.  Full-stack upload tests require a
    // GL context and live in the on-device integration suite.

    #[test]
    fn legacy_ahb_upload_obeys_the_session_circuit_breaker() {
        assert!(should_try_ahb_upload(true, true, true));
        assert!(!should_try_ahb_upload(false, true, true));
        assert!(!should_try_ahb_upload(true, false, true));
        assert!(!should_try_ahb_upload(true, true, false));
    }

    #[test]
    fn select_path_prefers_immutable_when_everything_available() {
        assert_eq!(
            TextureUploadPath::select(true, true, true),
            TextureUploadPath::PboImmutable
        );
    }

    #[test]
    fn select_path_falls_back_to_mutable_pbo_when_no_tex_storage() {
        assert_eq!(
            TextureUploadPath::select(true, false, true),
            TextureUploadPath::PboMutable
        );
    }

    #[test]
    fn select_path_uses_sync_when_pbo_unavailable() {
        assert_eq!(
            TextureUploadPath::select(false, true, true),
            TextureUploadPath::Synchronous
        );
        assert_eq!(
            TextureUploadPath::select(false, false, true),
            TextureUploadPath::Synchronous
        );
    }

    #[test]
    fn select_path_uses_sync_when_data_is_empty() {
        // Even on the fanciest GLES 3.0 driver: no data ⇒ no DMA, so
        // synchronous is both correct and allocation-free.
        for use_pbo in [false, true] {
            for has_ts in [false, true] {
                assert_eq!(
                    TextureUploadPath::select(use_pbo, has_ts, false),
                    TextureUploadPath::Synchronous,
                    "use_pbo={use_pbo} has_ts={has_ts}",
                );
            }
        }
    }

    #[test]
    fn select_path_is_deterministic_across_identical_inputs() {
        // Simple sanity check: same inputs always yield the same path,
        // i.e. the function is pure.
        for _ in 0..10 {
            assert_eq!(
                TextureUploadPath::select(true, true, true),
                TextureUploadPath::PboImmutable
            );
        }
    }
}
