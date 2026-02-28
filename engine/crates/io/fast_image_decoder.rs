//! Fast image decoder using optimized libraries.
//!
//! Uses `zune-image` for fast decoding with automatic RGBA conversion,
//! with fallback to `image` crate for unsupported formats.
//! On Android, a platform-native decoder (BitmapFactory via JNI) can be
//! registered at init time via `register_platform_decoder()`.

use std::sync::{Arc, OnceLock};

use shared::{
    error::{EngineError, ErrorCode},
    protocol::io_cmd::NormalizedImage,
};

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
    // Try platform-native decoder first (e.g., Android BitmapFactory)
    if let Some(decoder) = PLATFORM_DECODER.get() {
        match decoder(data) {
            Ok(img) => return Ok(img),
            Err(_platform_err) => {
                // When Rust decoders are compiled in, log and fall through.
                // Otherwise (e.g., Android build) return the platform error directly
                // so the caller sees the real failure reason.
                #[cfg(feature = "rust-image-decode")]
                {
                    tracing::warn!("platform decoder failed, falling back to Rust decoders: {_platform_err}");
                }
                #[cfg(not(feature = "rust-image-decode"))]
                {
                    return Err(_platform_err);
                }
            }
        }
    }

    #[cfg(feature = "rust-image-decode")]
    {
        // Try zune-image first (handles JPEG, PNG, BMP, etc. with SIMD optimization)
        match decode_with_zune(data) {
            Ok(img) => Ok(img),
            Err(_) => decode_with_image_crate(data),
        }
    }

    #[cfg(not(feature = "rust-image-decode"))]
    {
        Err(EngineError::new(ErrorCode::ImageReadError)
            .with_detail("no image decoder available (platform decoder not registered)"))
    }
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
        return Err(EngineError::new(ErrorCode::ImageReadError).with_detail(format!(
            "zune: expected {} bytes, got {}",
            expected_len,
            rgba.len()
        )));
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
