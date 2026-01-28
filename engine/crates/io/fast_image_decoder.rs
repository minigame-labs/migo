//! Fast image decoder using optimized libraries.
//!
//! Uses `zune-image` for fast decoding with automatic RGBA conversion,
//! with fallback to `image` crate for unsupported formats.

use shared::{
    error::{EngineError, ErrorCode},
    protocol::io_cmd::NormalizedImage,
};

/// Decode image data to RGBA format using the fastest available decoder.
///
/// # Arguments
/// * `data` - Raw image file bytes
/// * `_path_hint` - Optional path for extension-based format detection (unused, magic bytes preferred)
///
/// # Returns
/// `NormalizedImage` with RGBA8 pixel data
pub fn decode_image_fast(data: &[u8], _path_hint: Option<&str>) -> Result<NormalizedImage, EngineError> {
    // Try zune-image first (handles JPEG, PNG, BMP, etc. with SIMD optimization)
    match decode_with_zune(data) {
        Ok(img) => Ok(img),
        Err(_) => decode_with_image_crate(data),
    }
}

/// Decode using zune-image (fast, SIMD-optimized).
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
        EngineError::new(ErrorCode::ImageReadError)
            .with_detail("zune: no image data")
    })?;

    // Verify we got RGBA (4 bytes per pixel)
    let expected_len = (width * height * 4) as usize;
    if rgba.len() != expected_len {
        return Err(EngineError::new(ErrorCode::ImageReadError)
            .with_detail(format!(
                "zune: expected {} bytes, got {}",
                expected_len,
                rgba.len()
            )));
    }

    Ok(NormalizedImage {
        width: width as u32,
        height: height as u32,
        rgba,
    })
}

/// Decode image using the `image` crate (fallback).
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
        rgba,
    })
}
