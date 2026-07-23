//! Ingest-time image transcoding: PNG/JPEG in a package become an ETC2/KTX2
//! sidecar the runtime loads without decoding.
//!
//! The runtime half is already in place. `VARIANT_EXTENSIONS` lists `ktx2`
//! first, so `image_ops`'s companion probe loads `sprite.ktx2` in preference to
//! `sprite.png` the moment one exists, and `fast_image_decoder` recognises the
//! KTX2 container and hands its blocks straight to `glCompressedTexImage2D`.
//! What was missing is anything that *produces* the sidecar, which is this.
//!
//! The original image is always kept. The `.ktx2` sits beside it, so a device
//! without ETC2 (an ES 2.0 GPU), `getImageData`, and any path that needs real
//! RGBA still resolve the original through the same companion list. The sidecar
//! is an optimisation the runtime opts into per device, never a replacement.

use crate::etc2::{encode_etc2_rgb, VK_FORMAT_ETC2_R8G8B8_UNORM_BLOCK};
use crate::fast_image_decoder::decode_image_fast;
use crate::ktx2::write_ktx2;

/// A transcoded sidecar: the entry name to store it under and its KTX2 bytes.
pub(crate) struct TranscodedSidecar {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Whether an entry name looks like a source image this can transcode.
///
/// Matched by extension, case-insensitively. A file whose bytes are not
/// actually a decodable image is handled by `transcode_image` returning `None`
/// after the decode fails, so a misnamed entry costs a failed decode, never a
/// broken package.
pub(crate) fn is_transcodable_image(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".bmp")
}

/// Build the `.ktx2` sidecar name for a source image path: replace the last
/// extension with `ktx2`. `img/bg.png` -> `img/bg.ktx2`.
///
/// This must match how the runtime derives companions: `path_stem` +
/// `.ktx2`. Keeping the derivation here identical is what makes the sidecar
/// discoverable.
fn sidecar_name(name: &str) -> String {
    match name.rfind('.') {
        // Only treat a dot in the final path segment as an extension, so a
        // directory like `a.b/c` (no dot in `c`) is not truncated.
        Some(dot) if !name[dot..].contains('/') => format!("{}.ktx2", &name[..dot]),
        _ => format!("{name}.ktx2"),
    }
}

/// Transcode one image entry's bytes to an ETC2/KTX2 sidecar, or `None` when it
/// should be left alone.
///
/// Returns `None` — keep only the original — when:
/// - the bytes do not decode as an image;
/// - either dimension is not a multiple of 4. ETC2 has no partial blocks, and
///   padding would change the pixels the content addresses by index; the
///   original stays and the runtime decodes it the normal way.
///
/// The original entry is always written by the caller regardless; this only
/// decides whether a sidecar accompanies it.
pub(crate) fn transcode_image(name: &str, bytes: &[u8]) -> Option<TranscodedSidecar> {
    let image = decode_image_fast(bytes, Some(name)).ok()?;
    if image.width == 0 || image.height == 0 || image.width % 4 != 0 || image.height % 4 != 0 {
        return None;
    }
    let blocks = encode_etc2_rgb(&image.rgba, image.width, image.height).ok()?;
    let container = write_ktx2(
        VK_FORMAT_ETC2_R8G8B8_UNORM_BLOCK,
        image.width,
        image.height,
        &blocks,
    );
    Some(TranscodedSidecar {
        name: sidecar_name(name),
        bytes: container,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ktx2::{parse_ktx2, VkFormat};

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&[(x * 8) as u8, (y * 8) as u8, 0x40, 0xFF]);
            }
        }
        let buffer = image::RgbaImage::from_raw(width, height, rgba).unwrap();
        let mut out = std::io::Cursor::new(Vec::new());
        buffer
            .write_to(&mut out, image::ImageFormat::Png)
            .expect("encode test PNG");
        out.into_inner()
    }

    #[test]
    fn extensions_are_matched_case_insensitively() {
        assert!(is_transcodable_image("a.png"));
        assert!(is_transcodable_image("A.PNG"));
        assert!(is_transcodable_image("dir/b.JpG"));
        assert!(!is_transcodable_image("a.ktx2"));
        assert!(!is_transcodable_image("code/main.js"));
        assert!(!is_transcodable_image("noext"));
    }

    #[test]
    fn sidecar_name_replaces_only_the_final_extension() {
        assert_eq!(sidecar_name("img/bg.png"), "img/bg.ktx2");
        assert_eq!(sidecar_name("a.b/c.jpeg"), "a.b/c.ktx2");
        // A directory dot with no file extension must not be truncated.
        assert_eq!(sidecar_name("a.b/c"), "a.b/c.ktx2");
    }

    #[test]
    fn an_aligned_png_produces_a_parseable_etc2_ktx2() {
        let bytes = png(16, 16);
        let sidecar = transcode_image("img/bg.png", &bytes).expect("aligned image transcodes");
        assert_eq!(sidecar.name, "img/bg.ktx2");

        let parsed = parse_ktx2(&sidecar.bytes).expect("runtime parser accepts the sidecar");
        assert_eq!(parsed.header.format, VkFormat::Etc2R8G8B8UnormBlock);
        assert_eq!(parsed.header.width, 16);
        assert_eq!(parsed.header.height, 16);
        // 4x4 blocks, 8 bytes each: (16/4)^2 * 8.
        assert_eq!(parsed.data.len(), (16 / 4) * (16 / 4) * 8);
    }

    #[test]
    fn a_non_aligned_image_is_left_to_the_original() {
        // 14 is not a multiple of 4: ETC2 cannot encode it without padding, so
        // no sidecar is produced and the original PNG carries the asset.
        let bytes = png(14, 16);
        assert!(transcode_image("sprite.png", &bytes).is_none());
    }

    #[test]
    fn non_image_bytes_yield_no_sidecar() {
        assert!(transcode_image("data.png", b"this is not a PNG").is_none());
    }
}
