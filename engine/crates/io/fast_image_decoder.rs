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
    protocol::io_cmd::{AhbImage, NormalizedImage},
};

use crate::ktx2;

/// External RGBA decoder hook (legacy). Platform code calls
/// `register_platform_decoder()` at init time; kept for compatibility
/// and as the fallback when the AHB hook fails or isn't registered.
static PLATFORM_DECODER: OnceLock<fn(&[u8]) -> Result<NormalizedImage, EngineError>> =
    OnceLock::new();

/// Zero-copy AHB decoder hook. Android's `NativeExports.decodeImageAhb`
/// hands back an `AHardwareBuffer*`; the Rust side wraps it as an
/// [`AhbImage`] and the renderer imports it via `eglCreateImageKHR`
/// without touching CPU pixel bytes.
///
/// When this hook is registered **and** succeeds, [`decode_image_fast`]
/// returns the AHB variant; callers who need RGBA call `into_rgba()`
/// explicitly.  The hook may fail on a per-image basis (e.g. decoder
/// OOM, AHB alloc refused by driver); on failure we transparently
/// retry via [`PLATFORM_DECODER`] so the caller sees either a valid
/// image or a single error, never a silent downgrade.
static PLATFORM_AHB_DECODER: OnceLock<fn(&[u8]) -> Result<AhbImage, EngineError>> =
    OnceLock::new();

pub fn register_platform_decoder(f: fn(&[u8]) -> Result<NormalizedImage, EngineError>) {
    if PLATFORM_DECODER.set(f).is_ok() {
        tracing::info!("platform image decoder registered");
    } else {
        tracing::warn!("platform image decoder already registered, ignoring");
    }
}

pub fn register_platform_ahb_decoder(f: fn(&[u8]) -> Result<AhbImage, EngineError>) {
    if PLATFORM_AHB_DECODER.set(f).is_ok() {
        tracing::info!("platform AHB image decoder registered");
    } else {
        tracing::warn!("platform AHB image decoder already registered, ignoring");
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

/// Decode to the best available representation: prefers the AHB
/// zero-copy path when registered (Android API 28+ via
/// [`register_platform_ahb_decoder`]), otherwise falls through to
/// [`decode_image_fast`] and wraps the result as
/// [`shared::protocol::io_cmd::DecodedImage::Rgba`].
///
/// Callers that know they'll need CPU-side RGBA (e.g. `getImageData`,
/// resize, crop) can still call [`decode_image_fast`] directly to
/// skip the AHB-to-RGBA downgrade round-trip; everyone else should
/// prefer this entry so fast GPU-side uploads happen automatically
/// when supported.
pub fn decode_image_to_any(
    data: &[u8],
    path_hint: Option<&str>,
) -> Result<shared::protocol::io_cmd::DecodedImage, EngineError> {
    use shared::protocol::io_cmd::DecodedImage;

    if let Some(ahb_decoder) = PLATFORM_AHB_DECODER.get() {
        match ahb_decoder(data) {
            Ok(ahb) => return Ok(DecodedImage::HardwareBuffer(ahb)),
            Err(e) => {
                // AHB decode failed (OOM, AHB alloc refused, API<30,
                // etc.). Bump the observability counter and fall
                // through — the caller still gets a valid image via
                // the RGBA path.
                shared::stats::io_metrics_global()
                    .decoder_fallback_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tracing::debug!("AHB decode failed, falling back to RGBA: {e:?}");
            }
        }
    }

    decode_image_fast(data, path_hint).map(DecodedImage::Rgba)
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
/// Upper bound on a cropped RGBA buffer (128 MiB). Matches the soft
/// image byte budget; callers get a structured error instead of an
/// OOM abort when a game passes in a pathological `(sw, sh)`.
pub const MAX_SUBRECT_BYTES: usize = 128 * 1024 * 1024;

/// Crop `img` to the pixel rectangle `(sx, sy, sw, sh)`, with
/// WHATWG ImageBitmap out-of-bounds semantics: regions that fall
/// outside the source image are filled with transparent black
/// (`rgba(0, 0, 0, 0)`).
///
/// Returns a fresh `NormalizedImage` sized `sw x sh`.  When the
/// clipped region covers the whole source and has identical
/// dimensions, the input is returned unchanged (no allocation).
///
/// # Errors
/// * `InvalidArgument` when `sw > i32::MAX`, `sh > i32::MAX`, or
///   `sw * sh * 4` would overflow `usize`. The `i32::MAX` guard
///   prevents a `u32 -> i32` negative wrap downstream.
/// * `OutOfMemory` when the required buffer exceeds
///   [`MAX_SUBRECT_BYTES`].
///
/// Unlike [`resize_image`] this does not depend on the optional
/// `rust-image-decode` feature — crop is a pure CPU operation on
/// the RGBA buffer we always hold.
pub fn crop_image(
    img: NormalizedImage,
    sx: i32,
    sy: i32,
    sw: u32,
    sh: u32,
) -> Result<NormalizedImage, EngineError> {
    // Guard against u32 -> i32 wrap: later math casts `sw as i32`,
    // which silently produces a negative number for sw > i32::MAX.
    if sw > i32::MAX as u32 || sh > i32::MAX as u32 {
        return Err(EngineError::new(ErrorCode::InvalidArgument)
            .with_detail(format!("crop: size {}x{} exceeds i32::MAX", sw, sh)));
    }

    // Fast path: no-op crop.
    if sx == 0 && sy == 0 && sw == img.width && sh == img.height {
        return Ok(img);
    }

    // Checked allocation size. `sw`, `sh` are at most i32::MAX here,
    // so these multiplications only overflow usize on 32-bit targets.
    let pixels = (sw as usize).checked_mul(sh as usize).ok_or_else(|| {
        EngineError::new(ErrorCode::InvalidArgument)
            .with_detail(format!("crop: {}*{} pixel count overflows usize", sw, sh))
    })?;
    let bytes = pixels.checked_mul(4).ok_or_else(|| {
        EngineError::new(ErrorCode::InvalidArgument)
            .with_detail(format!("crop: {} pixels * 4 bytes overflows usize", pixels))
    })?;
    if bytes > MAX_SUBRECT_BYTES {
        return Err(EngineError::new(ErrorCode::OutOfMemory).with_detail(format!(
            "crop: {}x{} ({} bytes) exceeds MAX_SUBRECT_BYTES={}",
            sw, sh, bytes, MAX_SUBRECT_BYTES
        )));
    }

    let out_w = sw as i32;
    let out_h = sh as i32;
    let mut out = vec![0u8; bytes];

    // Compute intersection of destination rect with source image.
    let src_w = img.width as i32;
    let src_h = img.height as i32;
    let intersect_x0 = sx.max(0);
    let intersect_y0 = sy.max(0);
    // Use saturating arithmetic for sx+out_w so very large (but valid)
    // sx values don't wrap past i32::MAX before `.min(src_w)` clamps.
    let intersect_x1 = sx.saturating_add(out_w).min(src_w);
    let intersect_y1 = sy.saturating_add(out_h).min(src_h);
    if intersect_x1 > intersect_x0 && intersect_y1 > intersect_y0 {
        let row_bytes_src = (src_w as usize) * 4;
        let row_bytes_dst = (out_w as usize) * 4;
        for y in intersect_y0..intersect_y1 {
            let src_row_start = (y as usize) * row_bytes_src
                + (intersect_x0 as usize) * 4;
            let src_row_end = (y as usize) * row_bytes_src
                + (intersect_x1 as usize) * 4;
            let dst_y = (y - sy) as usize;
            let dst_x = (intersect_x0 - sx) as usize;
            let dst_start = dst_y * row_bytes_dst + dst_x * 4;
            let copy_len = src_row_end - src_row_start;
            out[dst_start..dst_start + copy_len]
                .copy_from_slice(&img.rgba[src_row_start..src_row_end]);
        }
    }

    Ok(NormalizedImage {
        width: sw,
        height: sh,
        rgba: std::sync::Arc::new(out),
    })
}

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

    // Hand the RGBA bytes to `image` without copying when possible.
    //
    // `Arc::try_unwrap` succeeds when we are the sole owner — the
    // common path: the image came straight from the decoder and
    // hasn't been handed out of the pipeline yet.  When the Arc has
    // outstanding refs (cache hit, shared source) we fall back to
    // `.to_vec()` which is the minimum-necessary copy.
    let w = img.width;
    let h = img.height;
    let raw: Vec<u8> = Arc::try_unwrap(img.rgba).unwrap_or_else(|arc| (*arc).clone());
    let src = match image::RgbaImage::from_raw(w, h, raw) {
        Some(s) => s,
        None => {
            tracing::warn!("resize_image: invalid buffer size {w}x{h}");
            // We consumed `img.rgba` above; rebuild a sentinel image
            // rather than returning the now-empty `img`.
            return NormalizedImage {
                width: w,
                height: h,
                rgba: Arc::new(Vec::new()),
            };
        }
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

#[cfg(test)]
mod crop_tests {
    use super::*;
    use std::sync::Arc;

    fn checkerboard(w: u32, h: u32) -> NormalizedImage {
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let on = (x + y) % 2 == 0;
                rgba[i] = if on { 255 } else { 0 };
                rgba[i + 1] = if on { 255 } else { 0 };
                rgba[i + 2] = if on { 255 } else { 0 };
                rgba[i + 3] = 255;
            }
        }
        NormalizedImage {
            width: w,
            height: h,
            rgba: Arc::new(rgba),
        }
    }

    #[test]
    fn crop_identity_returns_input() {
        let img = checkerboard(4, 4);
        let out = crop_image(img.clone(), 0, 0, 4, 4).expect("identity crop");
        assert_eq!(out.width, 4);
        assert_eq!(out.height, 4);
        assert_eq!(&*out.rgba, &*img.rgba);
    }

    #[test]
    fn crop_center_region_extracts_pixels_correctly() {
        // Source is a 4x4 checkerboard starting "on" at (0,0).
        // Extract the 2x2 from (1,1) — should start "on" at (1,1)
        // because (x+y) = 2, even.
        let img = checkerboard(4, 4);
        let out = crop_image(img, 1, 1, 2, 2).expect("center crop");
        assert_eq!(out.width, 2);
        assert_eq!(out.height, 2);
        // Top-left pixel of output = source(1,1): on.
        assert_eq!(out.rgba[0], 255);
        // (0,1) of output = source(1,2): off.
        let row1_off = (1 * 2 * 4) as usize;
        assert_eq!(out.rgba[row1_off + 0], 0);
    }

    #[test]
    fn crop_entirely_out_of_bounds_returns_transparent_black() {
        let img = checkerboard(4, 4);
        let out = crop_image(img, 10, 10, 3, 3).expect("oob crop");
        assert_eq!(out.width, 3);
        assert_eq!(out.height, 3);
        assert!(out.rgba.iter().all(|&b| b == 0));
    }

    #[test]
    fn crop_partially_out_of_bounds_fills_missing_with_transparent_black() {
        // Source 4x4, extract 6x6 starting at (-1, -1).  The 5x5 region
        // from (0,0) to (4,4) should be the source checker; the rim
        // is zeroed.
        let img = checkerboard(4, 4);
        let out = crop_image(img, -1, -1, 6, 6).expect("partial oob crop");
        assert_eq!(out.width, 6);
        assert_eq!(out.height, 6);
        // Top-left corner (output pixel (0,0) maps to source(-1,-1)
        // which is out of bounds) should be zero.
        assert_eq!(&out.rgba[0..4], &[0, 0, 0, 0]);
        // Output pixel (1,1) maps to source(0,0): on, so RGBA =
        // (255,255,255,255).
        let idx = (1 * 6 + 1) * 4;
        assert_eq!(&out.rgba[idx..idx + 4], &[255, 255, 255, 255]);
    }

    #[test]
    fn crop_rejects_size_above_i32_max() {
        // `sw as i32` would wrap to a negative value downstream; we
        // must reject before that.  Use 1-pixel tall so the pixel
        // count itself doesn't overflow first.
        let img = checkerboard(4, 4);
        let err = crop_image(img, 0, 0, (i32::MAX as u32) + 1, 1)
            .expect_err("i32::MAX+1 must be rejected");
        assert_eq!(err.code, ErrorCode::InvalidArgument);
    }

    #[test]
    fn crop_rejects_pixel_count_overflow() {
        // On 64-bit targets usize is 64-bit so `sw * sh` only overflows
        // above ~4 billion × ~4 billion; we already reject those via
        // the i32::MAX guard.  On 32-bit targets usize is 32-bit and
        // `(0x1_0000 * 0x1_0000)` overflows. Either way the caller
        // must see an error, not a panic.
        let img = checkerboard(4, 4);
        // 65536 x 65536 x 4 = 16 GiB; exceeds MAX_SUBRECT_BYTES on
        // every target, so this also validates the size-cap path.
        let err = crop_image(img, 0, 0, 65_536, 65_536)
            .expect_err("huge crop must be rejected");
        assert!(
            matches!(err.code, ErrorCode::InvalidArgument | ErrorCode::OutOfMemory),
            "unexpected code: {:?}",
            err.code
        );
    }

    #[test]
    fn crop_rejects_above_max_subrect_bytes() {
        // 8192 x 8192 x 4 = 256 MiB > MAX_SUBRECT_BYTES (128 MiB).
        let img = checkerboard(4, 4);
        let err = crop_image(img, 0, 0, 8192, 8192)
            .expect_err("> MAX_SUBRECT_BYTES must be rejected");
        assert_eq!(err.code, ErrorCode::OutOfMemory);
    }

    #[test]
    fn crop_at_max_subrect_bytes_succeeds() {
        // Largest allowed output: 128 MiB / 4 = 32 Mpx.
        // 8192 x 4096 x 4 = 128 MiB exactly. The source is tiny so
        // everything is out-of-bounds (transparent black), but the
        // allocation must succeed.
        let img = checkerboard(4, 4);
        let out = crop_image(img, 100, 100, 8192, 4096).expect("exact cap ok");
        assert_eq!(out.width, 8192);
        assert_eq!(out.height, 4096);
    }

    // ---- boundary-extension edges (P2-1) ----------------------------

    #[test]
    fn crop_extending_past_right_edge_copies_intersection_only() {
        // Source 4x4, request (sx=2, sy=0, sw=4, sh=2) — i.e. 4
        // columns wide starting at x=2.  Only x=[2,4) intersects
        // the source; the last 2 columns must be transparent
        // black, not wrapped or garbage.
        let img = checkerboard(4, 4);
        let out = crop_image(img, 2, 0, 4, 2).expect("rightward crop");
        assert_eq!(out.width, 4);
        assert_eq!(out.height, 2);
        // Output (2, 0) = source (4, 0) = OOB = zero.
        let idx = (0 * 4 + 2) * 4;
        assert_eq!(&out.rgba[idx..idx + 4], &[0, 0, 0, 0]);
        // Output (0, 0) = source (2, 0) = on (checker).
        assert_eq!(&out.rgba[0..4], &[255, 255, 255, 255]);
    }

    #[test]
    fn crop_extending_past_bottom_edge_copies_intersection_only() {
        // Source 4x4, request (sx=0, sy=3, sw=2, sh=4) — only the
        // last row of source overlaps.  Remaining 3 rows must be
        // transparent black (alpha=0), which is distinct from the
        // checker's "off" pixels (alpha=255 opaque black).
        let img = checkerboard(4, 4);
        let out = crop_image(img, 0, 3, 2, 4).expect("downward crop");
        assert_eq!(out.width, 2);
        assert_eq!(out.height, 4);
        // Output (0, 0) = source (0, 3) — checker "off" (x+y odd):
        // RGB=0 but alpha=255.  Proves the copy took the real row.
        assert_eq!(&out.rgba[0..4], &[0, 0, 0, 255]);
        // Output (1, 0) = source (1, 3) — checker "on" (x+y even).
        assert_eq!(&out.rgba[4..8], &[255, 255, 255, 255]);
        // Output (0, 1) = source (0, 4) = OOB.  Must be fully
        // transparent black; alpha=0 here distinguishes it from
        // the "off" pixels above.
        let idx = (1 * 2 + 0) * 4;
        assert_eq!(&out.rgba[idx..idx + 4], &[0, 0, 0, 0]);
        // All of rows 1..4 are OOB — every byte must be zero.
        for y in 1..4 {
            for x in 0..2 {
                let i = ((y * 2 + x) * 4) as usize;
                assert_eq!(
                    &out.rgba[i..i + 4],
                    &[0, 0, 0, 0],
                    "OOB pixel at ({}, {}) not transparent",
                    x, y
                );
            }
        }
    }

    #[test]
    fn crop_one_pixel_intersection_at_max_sx() {
        // sx = i32::MAX - 1 is a pathological-but-legal JS input;
        // saturating_add must keep us from wrapping to negative
        // before the src-bounds clamp.  Result is fully OOB.
        let img = checkerboard(4, 4);
        let out = crop_image(img, i32::MAX - 1, 0, 2, 2).expect("no panic on near-max sx");
        assert_eq!(out.width, 2);
        assert_eq!(out.height, 2);
        assert!(out.rgba.iter().all(|&b| b == 0));
    }
}
