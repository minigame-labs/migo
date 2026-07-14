//! Static-image decode directly into an API-26 `AHardwareBuffer`.
//!
//! The decoder is synchronous: encoded bytes are borrowed by Skia only until
//! `get_pixels_with_options` returns, and the buffer is synchronously unlocked
//! before the owned handle is published to the render pipeline.

use shared::{
    error::{EngineError, ErrorCode},
    protocol::{
        ahb::{AhbDesc, AhbError, AhbUsage, OwnedAhb},
        io_cmd::{AhbImage, MAX_IMAGE_PIXELS},
    },
};
use skia_safe::{AlphaType, Codec, ColorType, Data, EncodedOrigin, ImageInfo, codec};

const MIN_SKIA_DECODE_BUDGET: usize = 16 * 1024 * 1024;

fn image_error(detail: impl Into<String>) -> EngineError {
    EngineError::new(ErrorCode::ImageReadError).with_detail(detail)
}

fn ahb_error(stage: &'static str, error: AhbError) -> EngineError {
    image_error(format!("AHardwareBuffer {stage} failed: {error}"))
}

fn validate_dimensions(width: u32, height: u32) -> Result<usize, EngineError> {
    if width == 0 || height == 0 {
        return Err(image_error("Skia decoder returned zero image dimensions"));
    }
    let pixels = (width as u64)
        .checked_mul(height as u64)
        .ok_or_else(|| image_error("Skia decoder image dimensions overflow"))?;
    if pixels > MAX_IMAGE_PIXELS {
        return Err(EngineError::new(ErrorCode::OutOfMemory).with_detail(format!(
            "image {width}x{height} ({pixels} px) exceeds MAX_IMAGE_PIXELS ({MAX_IMAGE_PIXELS} px)"
        )));
    }
    (pixels as usize)
        .checked_mul(4)
        .ok_or_else(|| image_error("Skia decoder RGBA byte size overflows usize"))
}

/// Decode the first static frame into a GPU-importable, straight-alpha RGBA8
/// AHB. Any error is intentionally recoverable by `io::decode_image_to_any`,
/// which retries through the platform RGBA decoder.
pub fn decode_image_to_ahb(data: &[u8]) -> Result<AhbImage, EngineError> {
    if data.is_empty() {
        return Err(image_error("Skia decoder rejected empty input"));
    }

    // SAFETY: Skia does not own this memory. `codec` and its Data reference are
    // dropped before this synchronous function returns, while `data` remains
    // borrowed for the entire call. No decoder state escapes this function.
    let skia_data = unsafe { Data::new_bytes(data) };
    // The workspace deliberately builds Skia with a lean codec feature set.
    // Use its compiled registry so this module never creates an unresolved
    // reference to an excluded codec; unsupported formats take the platform
    // fallback instead of increasing every product's native binary.
    #[allow(deprecated)]
    let mut codec = Codec::from_data(skia_data)
        .ok_or_else(|| image_error("unsupported format: no linked Skia decoder accepted input"))?;

    let dimensions = codec.dimensions();
    let width = u32::try_from(dimensions.width)
        .map_err(|_| image_error("Skia decoder returned a negative image width"))?;
    let height = u32::try_from(dimensions.height)
        .map_err(|_| image_error("Skia decoder returned a negative image height"))?;
    let decoded_bytes = validate_dimensions(width, height)?;

    if codec.origin() != EncodedOrigin::TopLeft {
        return Err(image_error(format!(
            "unsupported format orientation {:?}; RGBA fallback must apply EXIF",
            codec.origin()
        )));
    }

    let ahb = OwnedAhb::allocate(AhbDesc::rgba_sampled_cpu_decode(width, height))
        .map_err(|error| ahb_error("allocation", error))?;
    let mut lock = ahb
        .lock_cpu(AhbUsage::CPU_WRITE_RARELY)
        .map_err(|error| ahb_error("CPU write lock", error))?;

    let decode_attempt = (|| {
        let image_info = ImageInfo::new(dimensions, ColorType::RGBA8888, AlphaType::Unpremul, None);
        let row_bytes = lock.stride_bytes();
        if !image_info.valid_row_bytes(row_bytes) {
            return Err(image_error(format!(
                "AHardwareBuffer row stride {row_bytes} is invalid for {width}x{height} RGBA8"
            )));
        }
        let required_bytes = image_info.compute_byte_size(row_bytes);
        let allocation = lock
            .as_bytes_mut()
            .map_err(|error| ahb_error("mutable CPU view", error))?;
        let available_bytes = allocation.len();
        let pixels = allocation.get_mut(..required_bytes).ok_or_else(|| {
            image_error(format!(
                "AHardwareBuffer allocation is too small: {available_bytes} bytes available, {required_bytes} required"
            ))
        })?;
        let options = codec::Options {
            max_decode_memory: Some(decoded_bytes.max(MIN_SKIA_DECODE_BUDGET)),
            ..codec::Options::default()
        };
        Ok(codec.get_pixels_with_options(&image_info, pixels, row_bytes, Some(&options)))
    })();

    let unlock_result = lock.finish();
    let decode_result = match decode_attempt {
        Ok(result) => result,
        Err(mut error) => {
            if let Err(unlock_error) = unlock_result {
                let detail = error.detail.take().unwrap_or_else(|| error.msg.to_string());
                error.detail = Some(format!(
                    "{detail}; AHardwareBuffer unlock also failed: {unlock_error}"
                ));
            }
            return Err(error);
        }
    };
    if decode_result != codec::Result::Success {
        let unlock_detail = unlock_result
            .err()
            .map(|error| format!("; AHardwareBuffer unlock also failed: {error}"))
            .unwrap_or_default();
        return Err(image_error(format!(
            "Skia decoder failed: {}{unlock_detail}",
            codec::result_to_string(decode_result)
        )));
    }
    unlock_result.map_err(|error| ahb_error("synchronous unlock", error))?;

    Ok(AhbImage::new(width, height, ahb))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_validation_is_zero_and_overflow_safe() {
        assert_eq!(validate_dimensions(1, 1).unwrap(), 4);
        assert!(validate_dimensions(0, 1).is_err());
        assert!(validate_dimensions(1, 0).is_err());
        let over = u32::try_from(MAX_IMAGE_PIXELS).unwrap() + 1;
        let error = validate_dimensions(over, 1).unwrap_err();
        assert_eq!(error.code, ErrorCode::OutOfMemory);
    }

    #[test]
    fn only_top_left_origin_is_direct_decode_eligible() {
        assert_eq!(EncodedOrigin::DEFAULT, EncodedOrigin::TopLeft);
        for origin in [
            EncodedOrigin::TopRight,
            EncodedOrigin::BottomRight,
            EncodedOrigin::BottomLeft,
            EncodedOrigin::LeftTop,
            EncodedOrigin::RightTop,
            EncodedOrigin::RightBottom,
            EncodedOrigin::LeftBottom,
        ] {
            assert_ne!(origin, EncodedOrigin::TopLeft);
        }
    }
}
