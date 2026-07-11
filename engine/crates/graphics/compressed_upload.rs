#![allow(unsafe_op_in_unsafe_fn)]
//! GPU-direct upload of compressed textures (ETC2/ASTC).
//!
//! Bypasses RGBA decoding entirely -- the compressed block data from a KTX2
//! container is uploaded straight to the GPU via `glCompressedTexImage2D`.
//!
//! Requires OpenGL ES 3.0 for ETC2 (mandatory in the spec) and the
//! `GL_KHR_texture_compression_astc_ldr` extension for ASTC.

use glow::HasContext;

// ---------------------------------------------------------------------------
// GL compressed-format constants (not exposed by glow)
// ---------------------------------------------------------------------------

/// `GL_COMPRESSED_RGB8_ETC2`
const GL_COMPRESSED_RGB8_ETC2: u32 = 0x9274;
/// `GL_COMPRESSED_RGBA8_ETC2_EAC`
const GL_COMPRESSED_RGBA8_ETC2_EAC: u32 = 0x9278;
/// `GL_COMPRESSED_RGBA_ASTC_4x4_KHR`
const GL_COMPRESSED_RGBA_ASTC_4X4_KHR: u32 = 0x93B0;
/// `GL_COMPRESSED_RGBA_ASTC_6x6_KHR`
const GL_COMPRESSED_RGBA_ASTC_6X6_KHR: u32 = 0x93B4;
/// `GL_COMPRESSED_RGBA_ASTC_8x8_KHR`
const GL_COMPRESSED_RGBA_ASTC_8X8_KHR: u32 = 0x93B9;

/// Compressed texture format for GPU upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressedFormat {
    /// ETC2 RGB (no alpha). 0.5 bytes/pixel.
    Etc2Rgb,
    /// ETC2 RGBA (with alpha). 1 byte/pixel.
    Etc2Rgba,
    /// ASTC 4x4 block. 1 byte/pixel.
    Astc4x4,
    /// ASTC 6x6 block. ~0.44 bytes/pixel.
    Astc6x6,
    /// ASTC 8x8 block. 0.25 bytes/pixel.
    Astc8x8,
}

impl CompressedFormat {
    /// Try to map a Vulkan format code (from KTX2 header) to a CompressedFormat.
    pub fn from_vk_format(vk_format: u32) -> Option<Self> {
        match vk_format {
            147 => Some(Self::Etc2Rgb),  // VK_FORMAT_ETC2_R8G8B8_UNORM_BLOCK
            151 => Some(Self::Etc2Rgba), // VK_FORMAT_ETC2_R8G8B8A8_UNORM_BLOCK
            157 => Some(Self::Astc4x4),  // VK_FORMAT_ASTC_4x4_UNORM_BLOCK
            163 => Some(Self::Astc6x6),  // VK_FORMAT_ASTC_6x6_UNORM_BLOCK
            169 => Some(Self::Astc8x8),  // VK_FORMAT_ASTC_8x8_UNORM_BLOCK
            _ => None,
        }
    }

    /// Whether this format requires ASTC extension support.
    pub fn requires_astc(self) -> bool {
        matches!(self, Self::Astc4x4 | Self::Astc6x6 | Self::Astc8x8)
    }

    /// Map to the corresponding OpenGL ES internal format constant.
    pub fn gl_internal_format(self) -> u32 {
        match self {
            Self::Etc2Rgb => GL_COMPRESSED_RGB8_ETC2,
            Self::Etc2Rgba => GL_COMPRESSED_RGBA8_ETC2_EAC,
            Self::Astc4x4 => GL_COMPRESSED_RGBA_ASTC_4X4_KHR,
            Self::Astc6x6 => GL_COMPRESSED_RGBA_ASTC_6X6_KHR,
            Self::Astc8x8 => GL_COMPRESSED_RGBA_ASTC_8X8_KHR,
        }
    }

    /// Human-readable label for logging.
    pub fn label(self) -> &'static str {
        match self {
            Self::Etc2Rgb => "ETC2_RGB",
            Self::Etc2Rgba => "ETC2_RGBA",
            Self::Astc4x4 => "ASTC_4x4",
            Self::Astc6x6 => "ASTC_6x6",
            Self::Astc8x8 => "ASTC_8x8",
        }
    }

    /// Compressed-block footprint `(width, height)` in texels.
    fn block_dims(self) -> (u32, u32) {
        match self {
            Self::Etc2Rgb | Self::Etc2Rgba | Self::Astc4x4 => (4, 4),
            Self::Astc6x6 => (6, 6),
            Self::Astc8x8 => (8, 8),
        }
    }

    /// Bytes per compressed block.
    fn bytes_per_block(self) -> u64 {
        match self {
            Self::Etc2Rgb => 8,
            Self::Etc2Rgba | Self::Astc4x4 | Self::Astc6x6 | Self::Astc8x8 => 16,
        }
    }

    /// Exact byte length a tightly-packed level-0 image of `width`x`height`
    /// must have in this format. A malformed KTX2 can declare dimensions that
    /// don't match its level-0 byte length; validating against this lets the
    /// caller reject it with a structured error instead of letting the driver
    /// fail the upload with GL_INVALID_VALUE.
    pub fn expected_level0_bytes(self, width: u32, height: u32) -> u64 {
        let (bw, bh) = self.block_dims();
        let blocks_x = (width as u64).div_ceil(bw as u64);
        let blocks_y = (height as u64).div_ceil(bh as u64);
        blocks_x
            .saturating_mul(blocks_y)
            .saturating_mul(self.bytes_per_block())
    }
}

/// Cached compressed-format support flags, detected once at init.
#[derive(Debug, Clone, Copy)]
pub struct CompressedFormatSupport {
    pub etc2: bool,
    pub astc: bool,
}

// GPU format support globals are in shared::device::gpu_caps for cross-crate access.

impl CompressedFormatSupport {
    /// Detect compressed format support from the current GL context.
    /// Call once during initialization and cache the result.
    /// Sets the per-session `GpuCaps` (shared via `Arc` with the host thread).
    pub fn detect(gl: &glow::Context, gpu_caps: &shared::device::gpu_caps::GpuCaps) -> Self {
        let version = unsafe { gl.get_parameter_string(glow::VERSION) };
        let etc2 = version.contains("OpenGL ES 3.") || version.contains("OpenGL ES 4.");

        let extensions = unsafe { gl.get_parameter_string(glow::EXTENSIONS) };
        let astc = extensions.contains("GL_KHR_texture_compression_astc_ldr")
            || extensions.contains("GL_KHR_texture_compression_astc_hdr");

        // Set session-level caps so IO/JS threads see them via snapshot().
        gpu_caps.set(etc2, astc);

        Self { etc2, astc }
    }

    /// Check whether the given compressed format is supported.
    pub fn is_supported(&self, format: CompressedFormat) -> bool {
        match format {
            CompressedFormat::Etc2Rgb | CompressedFormat::Etc2Rgba => self.etc2,
            CompressedFormat::Astc4x4 | CompressedFormat::Astc6x6 | CompressedFormat::Astc8x8 => {
                self.astc
            }
        }
    }
}

/// Upload pre-compressed texture data directly to the GPU.
///
/// Creates a new GL texture, binds it, and calls `glCompressedTexImage2D`
/// with the supplied block data. Returns the texture handle on success.
///
/// # Safety
///
/// A valid GL context must be current on the calling thread.
///
/// # Arguments
///
/// * `gl` -- glow context with a current GL context.
/// * `format` -- the compressed format of the data.
/// * `width` -- texture width in pixels (must match the data).
/// * `height` -- texture height in pixels (must match the data).
/// * `data` -- raw compressed block data (e.g., from a KTX2 level 0).
/// * `supports_pbo` -- whether the context has pixel-buffer objects (ES3 or
///   `GL_NV_pixel_buffer_object`). When false (bare ES2) the PBO binding is
///   left untouched, since querying `PIXEL_UNPACK_BUFFER_BINDING` would raise
///   `GL_INVALID_ENUM` and no PBO can be bound on such a context anyway.
///
/// # Returns
///
/// `Some(texture)` on success, `None` if texture creation or upload fails.
pub fn upload_compressed_texture(
    gl: &glow::Context,
    format: CompressedFormat,
    width: u32,
    height: u32,
    data: &[u8],
    supports_pbo: bool,
) -> Option<glow::NativeTexture> {
    unsafe {
        let tex = gl.create_texture().ok()?;

        // This may run on a live WebGL/Canvas2D context, so save every
        // app-visible binding we clobber and restore it afterwards; leaving one
        // changed would corrupt WebGL-visible state and desync the state
        // tracker (which would then dedup a later re-bind that GL never applied).
        //   * TEXTURE_BINDING_2D: core since ES2, always saved/restored.
        //   * PIXEL_UNPACK_BUFFER: the enum is ES3 / GL_NV_pixel_buffer_object
        //     only; querying it on a bare ES2 context raises GL_INVALID_ENUM, so
        //     only touch it when PBOs exist. With one bound, the `data` slice
        //     below would otherwise be reinterpreted as an offset into it.
        let saved_texture = gl.get_parameter_i32(glow::TEXTURE_BINDING_2D) as u32;
        let saved_unpack_buffer = if supports_pbo {
            let prev = gl.get_parameter_buffer(glow::PIXEL_UNPACK_BUFFER_BINDING);
            gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
            Some(prev)
        } else {
            None
        };

        gl.bind_texture(glow::TEXTURE_2D, Some(tex));

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

        let internal_format = format.gl_internal_format();

        // For ETC2/ASTC block formats `imageSize` must equal the exact
        // format+dimension-derived byte count; glCompressedTexImage2D returns
        // GL_INVALID_VALUE on any mismatch (there is no "allocate storage
        // only" idiom for compressed textures the way NULL pixels work for
        // glTexImage2D). Pass the real `data.len()` and upload straight from
        // client memory.
        gl.compressed_tex_image_2d(
            glow::TEXTURE_2D,
            0,
            internal_format as i32,
            width as i32,
            height as i32,
            0,
            data.len() as i32,
            data,
        );

        let err = gl.get_error();

        // Restore the app-visible bindings on both success and failure paths.
        gl.bind_texture(
            glow::TEXTURE_2D,
            std::num::NonZeroU32::new(saved_texture).map(glow::NativeTexture),
        );
        if let Some(prev) = saved_unpack_buffer {
            gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, prev);
        }

        if err != glow::NO_ERROR {
            tracing::warn!(
                "compressed upload failed: format={} {}x{} {} bytes, GL error=0x{:X}",
                format.label(),
                width,
                height,
                data.len(),
                err,
            );
            gl.delete_texture(tex);
            return None;
        }

        tracing::debug!(
            "compressed upload: format={} {}x{} {} bytes",
            format.label(),
            width,
            height,
            data.len(),
        );

        Some(tex)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gl_internal_format_values() {
        assert_eq!(CompressedFormat::Etc2Rgb.gl_internal_format(), 0x9274);
        assert_eq!(CompressedFormat::Etc2Rgba.gl_internal_format(), 0x9278);
        assert_eq!(CompressedFormat::Astc4x4.gl_internal_format(), 0x93B0);
        assert_eq!(CompressedFormat::Astc6x6.gl_internal_format(), 0x93B4);
        assert_eq!(CompressedFormat::Astc8x8.gl_internal_format(), 0x93B9);
    }

    #[test]
    fn format_labels() {
        assert_eq!(CompressedFormat::Etc2Rgb.label(), "ETC2_RGB");
        assert_eq!(CompressedFormat::Etc2Rgba.label(), "ETC2_RGBA");
        assert_eq!(CompressedFormat::Astc4x4.label(), "ASTC_4x4");
        assert_eq!(CompressedFormat::Astc6x6.label(), "ASTC_6x6");
        assert_eq!(CompressedFormat::Astc8x8.label(), "ASTC_8x8");
    }

    #[test]
    fn expected_level0_bytes_block_math() {
        // ETC2 RGB: 4x4 block, 8 B/block.
        assert_eq!(CompressedFormat::Etc2Rgb.expected_level0_bytes(8, 8), 32);
        assert_eq!(CompressedFormat::Etc2Rgb.expected_level0_bytes(1, 1), 8);
        // ETC2 RGBA / ASTC 4x4: 4x4 block, 16 B/block.
        assert_eq!(CompressedFormat::Etc2Rgba.expected_level0_bytes(8, 8), 64);
        assert_eq!(CompressedFormat::Astc4x4.expected_level0_bytes(8, 8), 64);
        // ASTC 6x6: ceil(7/6)=2 per axis -> 4 blocks * 16 = 64.
        assert_eq!(CompressedFormat::Astc6x6.expected_level0_bytes(7, 7), 64);
        // ASTC 8x8: exact single block vs partial-edge round-up.
        assert_eq!(CompressedFormat::Astc8x8.expected_level0_bytes(8, 8), 16);
        assert_eq!(CompressedFormat::Astc8x8.expected_level0_bytes(9, 9), 64);
    }
}
