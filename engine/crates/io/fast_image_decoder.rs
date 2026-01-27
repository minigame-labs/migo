//! Fast image decoder using optimized libraries.
//!
//! Uses `zune-jpeg` and `zune-png` for JPEG/PNG (much faster than `image` crate),
//! with fallback to `image` crate for other formats.

use shared::{
    error::{EngineError, ErrorCode},
    protocol::io_cmd::NormalizedImage,
};
use zune_core::colorspace::ColorSpace;

/// Detected image format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
    Gif,
    WebP,
    Bmp,
    Unknown,
}

impl ImageFormat {
    /// Detect format from magic bytes.
    #[inline]
    pub fn from_magic(data: &[u8]) -> Self {
        if data.len() < 8 {
            return Self::Unknown;
        }

        // JPEG: FF D8 FF
        if data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
            return Self::Jpeg;
        }

        // PNG: 89 50 4E 47 0D 0A 1A 0A
        if data[0..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
            return Self::Png;
        }

        // GIF: GIF87a or GIF89a
        if data[0..6] == *b"GIF87a" || data[0..6] == *b"GIF89a" {
            return Self::Gif;
        }

        // WebP: RIFF....WEBP
        if data.len() >= 12
            && data[0..4] == *b"RIFF"
            && data[8..12] == *b"WEBP"
        {
            return Self::WebP;
        }

        // BMP: BM
        if data[0..2] == *b"BM" {
            return Self::Bmp;
        }

        Self::Unknown
    }

    /// Detect format from file extension.
    #[inline]
    pub fn from_extension(path: &str) -> Self {
        let path_lower = path.to_lowercase();
        if path_lower.ends_with(".jpg") || path_lower.ends_with(".jpeg") {
            Self::Jpeg
        } else if path_lower.ends_with(".png") {
            Self::Png
        } else if path_lower.ends_with(".gif") {
            Self::Gif
        } else if path_lower.ends_with(".webp") {
            Self::WebP
        } else if path_lower.ends_with(".bmp") {
            Self::Bmp
        } else {
            Self::Unknown
        }
    }
}

/// Decode JPEG using zune-jpeg (faster than image crate).
fn decode_jpeg_fast(data: &[u8]) -> Result<NormalizedImage, EngineError> {
    use zune_jpeg::JpegDecoder;

    let mut decoder = JpegDecoder::new(data);

    // Decode to RGB
    let pixels = decoder.decode().map_err(|e| {
        EngineError::new(ErrorCode::ImageReadError)
            .with_detail(format!("zune-jpeg decode error: {:?}", e))
    })?;

    let info = decoder.info().ok_or_else(|| {
        EngineError::new(ErrorCode::ImageReadError)
            .with_detail("zune-jpeg: no image info")
    })?;

    let width = info.width as u32;
    let height = info.height as u32;
    let colorspace = decoder.get_output_colorspace().unwrap_or(ColorSpace::RGB);

    // Convert to RGBA
    let rgba = normalize_to_rgba(&pixels, colorspace, width, height)?;

    Ok(NormalizedImage {
        width,
        height,
        rgba,
    })
}

/// Decode PNG using zune-png (faster than image crate).
fn decode_png_fast(data: &[u8]) -> Result<NormalizedImage, EngineError> {
    use zune_png::PngDecoder;

    let mut decoder = PngDecoder::new(data);

    let pixels = decoder.decode_raw().map_err(|e| {
        EngineError::new(ErrorCode::ImageReadError)
            .with_detail(format!("zune-png decode error: {:?}", e))
    })?;

    let info = decoder.get_info().ok_or_else(|| {
        EngineError::new(ErrorCode::ImageReadError)
            .with_detail("zune-png: no image info")
    })?;

    let width = info.width as u32;
    let height = info.height as u32;
    let colorspace = decoder.get_colorspace().unwrap_or(ColorSpace::RGBA);

    // Convert to RGBA
    let rgba = normalize_to_rgba(&pixels, colorspace, width, height)?;

    Ok(NormalizedImage {
        width,
        height,
        rgba,
    })
}

/// Decode image using the `image` crate (fallback for other formats).
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

/// Normalize decoded pixels to RGBA format.
fn normalize_to_rgba(
    pixels: &[u8],
    colorspace: ColorSpace,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, EngineError> {
    let pixel_count = (width as usize) * (height as usize);

    match colorspace {
        ColorSpace::RGBA => {
            // Already RGBA
            if pixels.len() == pixel_count * 4 {
                Ok(pixels.to_vec())
            } else {
                Err(EngineError::new(ErrorCode::ImageReadError)
                    .with_detail("RGBA pixel count mismatch"))
            }
        }
        ColorSpace::RGB => {
            // RGB -> RGBA
            if pixels.len() != pixel_count * 3 {
                return Err(EngineError::new(ErrorCode::ImageReadError)
                    .with_detail("RGB pixel count mismatch"));
            }
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for chunk in pixels.chunks_exact(3) {
                rgba.push(chunk[0]);
                rgba.push(chunk[1]);
                rgba.push(chunk[2]);
                rgba.push(255); // Alpha
            }
            Ok(rgba)
        }
        ColorSpace::Luma => {
            // Grayscale -> RGBA
            if pixels.len() != pixel_count {
                return Err(EngineError::new(ErrorCode::ImageReadError)
                    .with_detail("Luma pixel count mismatch"));
            }
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for &gray in pixels {
                rgba.push(gray);
                rgba.push(gray);
                rgba.push(gray);
                rgba.push(255);
            }
            Ok(rgba)
        }
        ColorSpace::LumaA => {
            // Grayscale+Alpha -> RGBA
            if pixels.len() != pixel_count * 2 {
                return Err(EngineError::new(ErrorCode::ImageReadError)
                    .with_detail("LumaA pixel count mismatch"));
            }
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for chunk in pixels.chunks_exact(2) {
                let gray = chunk[0];
                let alpha = chunk[1];
                rgba.push(gray);
                rgba.push(gray);
                rgba.push(gray);
                rgba.push(alpha);
            }
            Ok(rgba)
        }
        _ => {
            Err(EngineError::new(ErrorCode::ImageReadError)
                .with_detail(format!("unsupported colorspace: {:?}", colorspace)))
        }
    }
}

/// Decode image data to RGBA format using the fastest available decoder.
///
/// # Arguments
/// * `data` - Raw image file bytes
/// * `path_hint` - Optional path for extension-based format detection
///
/// # Returns
/// `NormalizedImage` with RGBA8 pixel data
pub fn decode_image_fast(data: &[u8], path_hint: Option<&str>) -> Result<NormalizedImage, EngineError> {
    // Try format detection from magic bytes first
    let format = ImageFormat::from_magic(data);

    // Fall back to extension if unknown
    let format = if format == ImageFormat::Unknown {
        path_hint.map(ImageFormat::from_extension).unwrap_or(ImageFormat::Unknown)
    } else {
        format
    };

    match format {
        ImageFormat::Jpeg => decode_jpeg_fast(data),
        ImageFormat::Png => decode_png_fast(data),
        // Use image crate for other formats
        _ => decode_with_image_crate(data),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_detection_jpeg() {
        let data = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46];
        assert_eq!(ImageFormat::from_magic(&data), ImageFormat::Jpeg);
    }

    #[test]
    fn test_format_detection_png() {
        let data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(ImageFormat::from_magic(&data), ImageFormat::Png);
    }

    #[test]
    fn test_format_from_extension() {
        assert_eq!(ImageFormat::from_extension("test.jpg"), ImageFormat::Jpeg);
        assert_eq!(ImageFormat::from_extension("test.JPEG"), ImageFormat::Jpeg);
        assert_eq!(ImageFormat::from_extension("test.png"), ImageFormat::Png);
        assert_eq!(ImageFormat::from_extension("test.PNG"), ImageFormat::Png);
        assert_eq!(ImageFormat::from_extension("test.gif"), ImageFormat::Gif);
        assert_eq!(ImageFormat::from_extension("test.webp"), ImageFormat::WebP);
        assert_eq!(ImageFormat::from_extension("test.txt"), ImageFormat::Unknown);
    }
}
