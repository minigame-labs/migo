//! Fast image decoder using optimized libraries.
//!
//! Uses `zune-image` for fast decoding with automatic RGBA conversion,
//! with fallback to `image` crate for unsupported formats.
//! On Android, a platform-native decoder (BitmapFactory via JNI) can be
//! registered at init time via `register_platform_decoder()`.

use std::sync::OnceLock;
#[cfg(feature = "rust-image-decode")]
use std::sync::Arc;

use shared::{
    error::{EngineError, ErrorCode},
    protocol::io_cmd::NormalizedImage,
};

use crate::ktx2;

/// External decoder hook (e.g., Android BitmapFactory via JNI).
/// Platform code calls `register_platform_decoder()` at init time.
static PLATFORM_DECODER: OnceLock<fn(&[u8]) -> Result<NormalizedImage, EngineError>> =
    OnceLock::new();

pub fn register_platform_decoder(f: fn(&[u8]) -> Result<NormalizedImage, EngineError>) {
    if PLATFORM_DECODER.set(f).is_ok() {
        tracing::info!("platform image decoder registered");
    } else {
        tracing::warn!("platform image decoder already registered, ignoring");
    }
}

/// Describes a compressed texture detected from file magic bytes.
///
/// When a file is identified as a KTX2 container holding ETC2 or ASTC data,
/// the caller should skip RGBA decoding and instead pass the raw compressed
/// data directly to the GPU via `glCompressedTexImage2D`.
#[derive(Debug, Clone)]
pub struct CompressedImageInfo {
    pub width: u32,
    pub height: u32,
    /// Vulkan format code as parsed from the KTX2 header.
    pub vk_format: ktx2::VkFormat,
    /// Byte offset of the compressed level 0 data within the original buffer.
    pub data_offset: usize,
    /// Byte length of the compressed level 0 data.
    pub data_len: usize,
}

/// Detect whether `data` is a KTX2 container holding a compressed GPU texture.
///
/// Returns `Some(CompressedImageInfo)` if the file is a valid KTX2 with a
/// recognized compressed format (ETC2 or ASTC). Returns `None` for regular
/// images (PNG, JPEG, etc.) that should go through normal RGBA decoding.
///
/// This function does **no allocation** -- it only inspects header bytes.
pub fn detect_compressed_format(data: &[u8]) -> Option<CompressedImageInfo> {
    if !ktx2::is_ktx2(data) {
        return None;
    }

    let ktx2_file = ktx2::parse_ktx2(data).ok()?;

    // Only return info for formats we can upload to GL.
    match ktx2_file.header.format {
        ktx2::VkFormat::Etc2R8G8B8UnormBlock
        | ktx2::VkFormat::Etc2R8G8B8A8UnormBlock
        | ktx2::VkFormat::Astc4x4UnormBlock
        | ktx2::VkFormat::Astc6x6UnormBlock
        | ktx2::VkFormat::Astc8x8UnormBlock => {}
        ktx2::VkFormat::Unknown(_) => return None,
    }

    // Compute the offset of the data slice within the original buffer.
    let data_ptr = ktx2_file.data.as_ptr() as usize;
    let base_ptr = data.as_ptr() as usize;
    let data_offset = data_ptr - base_ptr;

    Some(CompressedImageInfo {
        width: ktx2_file.header.width,
        height: ktx2_file.header.height,
        vk_format: ktx2_file.header.format,
        data_offset,
        data_len: ktx2_file.data.len(),
    })
}

/// Estimate the decoded RGBA byte size from image file headers without full
/// decoding.  Returns `width * height * 4` on success, or a conservative
/// fallback (16 MB = 2048x2048x4) if the header cannot be parsed.
///
/// This is cheap (~microseconds) and used by the IO byte budget to reserve
/// the right amount before committing to a full decode.
#[allow(dead_code)]
pub fn estimate_decoded_size(data: &[u8]) -> usize {
    const FALLBACK: usize = 2048 * 2048 * 4; // 16 MB
    const MAX_ESTIMATE: usize = 256 * 1024 * 1024; // 256 MB cap

    if let Some((w, h)) = probe_dimensions(data) {
        (w as usize)
            .checked_mul(h as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .map(|bytes| bytes.min(MAX_ESTIMATE))
            .unwrap_or(FALLBACK)
    } else {
        FALLBACK
    }
}

/// Try to read image dimensions from the file header.
#[allow(dead_code)]
fn probe_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    // KTX2: compressed texture container -- width/height in header.
    if ktx2::is_ktx2(data) {
        if let Ok(f) = ktx2::parse_ktx2(data) {
            return Some((f.header.width, f.header.height));
        }
    }

    // PNG: bytes 16..24 contain width (u32 BE) and height (u32 BE) in IHDR.
    if data.len() >= 24 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
        let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
        return Some((w, h));
    }

    // JPEG: scan for SOF0/SOF2 marker (0xFF 0xC0 or 0xFF 0xC2).
    if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xD8 {
        let mut i = 2;
        while i + 9 < data.len() {
            if data[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = data[i + 1];
            if marker == 0xC0 || marker == 0xC2 {
                // SOF: length(2) + precision(1) + height(2) + width(2)
                let h = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
                let w = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
                return Some((w, h));
            }
            // Skip segment: length is at data[i+2..i+4]
            if i + 3 < data.len() {
                let seg_len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
                i += 2 + seg_len;
            } else {
                break;
            }
        }
    }

    // BMP: bytes 18..26 contain width (i32 LE) and height (i32 LE).
    if data.len() >= 26 && &data[0..2] == b"BM" {
        let w = i32::from_le_bytes([data[18], data[19], data[20], data[21]]);
        let h = i32::from_le_bytes([data[22], data[23], data[24], data[25]]).abs();
        return Some((w as u32, h as u32));
    }

    // WebP: "RIFF" + size + "WEBP" + chunk. VP8 /VP8L/VP8X have dimensions.
    if data.len() >= 30 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        // VP8L (lossless): signature 0x2F at offset 12
        if data[12] == 0x56 && data[13] == 0x50 && data[14] == 0x38 && data[15] == 0x4C {
            // VP8L: width-1 at bits [0..14], height-1 at bits [14..28] of bytes 21..25
            if data.len() >= 25 {
                let b = u32::from_le_bytes([data[21], data[22], data[23], data[24]]);
                let w = (b & 0x3FFF) + 1;
                let h = ((b >> 14) & 0x3FFF) + 1;
                return Some((w, h));
            }
        }
    }

    None
}

/// Decode image data to RGBA format using the fastest available decoder.
///
/// # Arguments
/// * `data` - Raw image file bytes
/// * `_path_hint` - Optional path for extension-based format detection (unused, magic bytes preferred)
///
/// # Returns
/// `NormalizedImage` with RGBA8 pixel data
pub fn decode_image_fast(
    data: &[u8],
    _path_hint: Option<&str>,
) -> Result<NormalizedImage, EngineError> {
    // Priority: Rust-native decoders first (zero JNI, zero Java Heap),
    // platform decoder (BitmapFactory) as last resort.
    #[cfg(feature = "rust-image-decode")]
    {
        match decode_with_zune(data) {
            Ok(img) => return Ok(img),
            Err(_zune_err) => {
                // zune failed — try image crate before falling back to platform.
                match decode_with_image_crate(data) {
                    Ok(img) => return Ok(img),
                    Err(_img_err) => {
                        tracing::debug!(
                            "Rust decoders failed (zune: {_zune_err}, image: {_img_err}), trying platform"
                        );
                    }
                }
            }
        }
    }

    // Platform decoder fallback (e.g., Android BitmapFactory via JNI).
    if let Some(decoder) = PLATFORM_DECODER.get() {
        return decoder(data);
    }

    Err(EngineError::new(ErrorCode::ImageReadError)
        .with_detail("no image decoder available (all decoders failed or not registered)"))
}

/// Decode using zune-image (fast, SIMD-optimized).
#[cfg(feature = "rust-image-decode")]
fn decode_with_zune(data: &[u8]) -> Result<NormalizedImage, EngineError> {
    use std::io::Cursor;
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    use zune_image::image::Image;

    // Configure decoder options
    let options = DecoderOptions::new_fast();

    // Wrap data in a Cursor to provide Seek trait
    let cursor = Cursor::new(data);

    // Auto-detect format and decode
    let mut image = Image::read(cursor, options).map_err(|e| {
        EngineError::new(ErrorCode::ImageReadError)
            .with_detail(format!("zune decode error: {:?}", e))
    })?;

    // Convert to RGBA colorspace
    image.convert_color(ColorSpace::RGBA).map_err(|e| {
        EngineError::new(ErrorCode::ImageReadError)
            .with_detail(format!("zune color convert error: {:?}", e))
    })?;

    // Get dimensions
    let (width, height) = image.dimensions();

    // Flatten to RGBA bytes
    let rgba = image.flatten_to_u8().pop().ok_or_else(|| {
        EngineError::new(ErrorCode::ImageReadError).with_detail("zune: no image data")
    })?;

    // Verify we got RGBA (4 bytes per pixel)
    let expected_len = (width * height * 4) as usize;
    if rgba.len() != expected_len {
        return Err(
            EngineError::new(ErrorCode::ImageReadError).with_detail(format!(
                "zune: expected {} bytes, got {}",
                expected_len,
                rgba.len()
            )),
        );
    }

    Ok(NormalizedImage {
        width: width as u32,
        height: height as u32,
        rgba: Arc::new(rgba),
    })
}

/// Decode image using the `image` crate (fallback).
#[cfg(feature = "rust-image-decode")]
fn decode_with_image_crate(data: &[u8]) -> Result<NormalizedImage, EngineError> {
    use image::GenericImageView;

    let img = image::load_from_memory(data).map_err(|e| {
        EngineError::new(ErrorCode::ImageReadError)
            .with_detail(format!("image crate decode error: {}", e))
    })?;

    let (width, height) = img.dimensions();
    let rgba = img.into_rgba8().into_raw();

    Ok(NormalizedImage {
        width,
        height,
        rgba: Arc::new(rgba),
    })
}

/// Resize a decoded image to fit within `target_w x target_h`, preserving
/// aspect ratio. Uses bilinear filtering via the `image` crate.
///
/// If the image is already within the target dimensions, returns it unchanged.
#[cfg(feature = "rust-image-decode")]
pub fn resize_image(img: NormalizedImage, target_w: u32, target_h: u32) -> NormalizedImage {
    if img.width <= target_w && img.height <= target_h {
        return img;
    }

    // Compute aspect-preserving dimensions.
    let scale = f64::min(
        target_w as f64 / img.width as f64,
        target_h as f64 / img.height as f64,
    );
    let new_w = ((img.width as f64 * scale).round() as u32).max(1);
    let new_h = ((img.height as f64 * scale).round() as u32).max(1);

    let src = image::RgbaImage::from_raw(img.width, img.height, img.rgba.as_ref().clone());
    let Some(src) = src else {
        tracing::warn!(
            "resize_image: invalid buffer size {}x{} ({} bytes)",
            img.width,
            img.height,
            img.rgba.len()
        );
        return img;
    };

    let resized = image::imageops::resize(&src, new_w, new_h, image::imageops::FilterType::Triangle);
    NormalizedImage {
        width: new_w,
        height: new_h,
        rgba: Arc::new(resized.into_raw()),
    }
}

/// Fallback when `rust-image-decode` is disabled — returns the image unchanged.
#[cfg(not(feature = "rust-image-decode"))]
pub fn resize_image(img: NormalizedImage, _target_w: u32, _target_h: u32) -> NormalizedImage {
    img
}
