//! Ingest-time image transcoding: PNG/JPEG in a package become a compressed
//! KTX2 sidecar the runtime loads without decoding.
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

use shared::device::gpu_caps::GpuCapsSnapshot;

use crate::astc::{VK_FORMAT_ASTC_4X4_UNORM_BLOCK, encode_astc_4x4};
use crate::etc2::{
    VK_FORMAT_ETC2_R8G8B8_UNORM_BLOCK, VK_FORMAT_ETC2_R8G8B8A8_UNORM_BLOCK, encode_etc2_rgb,
    encode_etc2_rgba,
};
use crate::fast_image_decoder::decode_image_fast;
use crate::ktx2::write_ktx2_levels;

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
pub(crate) fn transcode_image(
    name: &str,
    bytes: &[u8],
    caps: GpuCapsSnapshot,
) -> Option<TranscodedSidecar> {
    let image = decode_image_fast(bytes, Some(name)).ok()?;
    if image.width == 0 || image.height == 0 || image.width % 4 != 0 || image.height % 4 != 0 {
        return None;
    }

    // A fully-opaque image uses ETC2 RGB, whose blocks are half the size; only
    // an image with real transparency pays for the EAC alpha block. Checking the
    // pixels rather than the file's colour type is what lets an RGBA PNG whose
    // alpha is all 255 still take the smaller format.
    let has_alpha = image.rgba.chunks_exact(4).any(|px| px[3] != 0xFF);

    // The format is chosen per image *and* per device, and the rule is that no
    // device is ever worse off than it was:
    //
    // - Opaque images stay ETC2 RGB. At half a byte per pixel it is smaller
    //   than any ASTC footprint this encoder produces, and every ES 3.0 device
    //   decodes it. ASTC would double the bytes for nothing.
    // - Images with alpha take ASTC 4x4 where the device decodes it. Both are
    //   one byte per pixel, so the size is identical, and ASTC's second weight
    //   plane reconstructs an alpha edge that does not follow the colour edge --
    //   which is what a sprite outline is -- where ETC2 RGBA shares one set of
    //   modifiers between colour and alpha.
    //
    // Choosing on the device is only possible because ingest runs there. A
    // build-time choice would have to ship both or pick the lowest common
    // denominator; this ships one file, sized the same either way.
    //
    // Caps that are not ready yet read as absent, which lands on ETC2 -- the
    // format every device can decode. An unknown answer has to fall to the
    // universal one, not the better one.
    let use_astc = has_alpha && caps.astc;

    let vk_format = if use_astc {
        VK_FORMAT_ASTC_4X4_UNORM_BLOCK
    } else if has_alpha {
        VK_FORMAT_ETC2_R8G8B8A8_UNORM_BLOCK
    } else {
        VK_FORMAT_ETC2_R8G8B8_UNORM_BLOCK
    };

    // The whole chain, not just the base level. A texture that ships one level
    // is sampled with that level at every scale: a minified sprite reads pixels
    // far apart in a full-resolution image, which aliases *and* thrashes the
    // texture cache. The extra levels cost about a third of the base level in
    // package bytes, once, instead of that cost every frame.
    //
    // The chain stops where the encoder would need padding (see `rgba_mip_chain`),
    // so it is often partial -- which the uploader handles by bounding
    // `TEXTURE_MAX_LEVEL` to what it was actually given.
    let chain = crate::mipmap::rgba_mip_chain(&image.rgba, image.width, image.height, 4);
    let mut encoded: Vec<Vec<u8>> = Vec::with_capacity(chain.len());
    for (level_rgba, level_width, level_height) in &chain {
        let blocks = if use_astc {
            encode_astc_4x4(level_rgba, *level_width, *level_height).ok()?
        } else if has_alpha {
            encode_etc2_rgba(level_rgba, *level_width, *level_height).ok()?
        } else {
            encode_etc2_rgb(level_rgba, *level_width, *level_height).ok()?
        };
        encoded.push(blocks);
    }
    let levels: Vec<&[u8]> = encoded.iter().map(Vec::as_slice).collect();

    let container = write_ktx2_levels(vk_format, image.width, image.height, &levels)?;
    Some(TranscodedSidecar {
        name: sidecar_name(name),
        bytes: container,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device that decodes ETC2 and not ASTC, which is the ES 3.0 floor and
    /// what every case here that does not say otherwise assumes.
    fn no_astc() -> GpuCapsSnapshot {
        GpuCapsSnapshot {
            etc2: true,
            astc: false,
            ahb: false,
        }
    }

    /// A device that decodes both.
    fn with_astc() -> GpuCapsSnapshot {
        GpuCapsSnapshot {
            etc2: true,
            astc: true,
            ahb: false,
        }
    }
    use crate::ktx2::{VkFormat, parse_ktx2};

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
    fn a_sidecar_carries_the_whole_mip_chain_not_just_the_base() {
        // Without this the chain code could quietly emit one level and every
        // other test here would still pass: they only ask whether the sidecar
        // parses, and a one-level sidecar parses fine.
        let png = png(64, 64);
        let sidecar = transcode_image("a.png", &png, no_astc()).expect("64x64 transcodes");
        let parsed = crate::ktx2::parse_ktx2(&sidecar.bytes).expect("parses");

        // 64 -> 32 -> 16 -> 8 -> 4, stopping where ETC2 would need padding.
        assert_eq!(parsed.header.mip_levels, 5);
        let levels: Vec<&[u8]> = parsed.levels().collect();
        assert_eq!(levels.len(), 5);

        // Each level is a quarter of the previous one's blocks, which is the
        // arithmetic that makes a chain cost about a third extra rather than
        // double.
        for pair in levels.windows(2) {
            assert_eq!(
                pair[1].len() * 4,
                pair[0].len(),
                "each level must be a quarter of the one above it"
            );
        }
    }

    #[test]
    fn a_sidecar_too_small_to_halve_still_ships_its_base_level() {
        let png = png(4, 4);
        let sidecar = transcode_image("tiny.png", &png, no_astc()).expect("4x4 transcodes");
        let parsed = crate::ktx2::parse_ktx2(&sidecar.bytes).expect("parses");
        assert_eq!(parsed.header.mip_levels, 1);
    }

    #[test]
    fn an_aligned_png_produces_a_parseable_etc2_ktx2() {
        let bytes = png(16, 16);
        let sidecar =
            transcode_image("img/bg.png", &bytes, no_astc()).expect("aligned image transcodes");
        assert_eq!(sidecar.name, "img/bg.ktx2");

        let parsed = parse_ktx2(&sidecar.bytes).expect("runtime parser accepts the sidecar");
        assert_eq!(parsed.header.format, VkFormat::Etc2R8G8B8UnormBlock);
        assert_eq!(parsed.header.width, 16);
        assert_eq!(parsed.header.height, 16);
        // 4x4 blocks, 8 bytes each: (16/4)^2 * 8.
        assert_eq!(parsed.data.len(), (16 / 4) * (16 / 4) * 8);
    }

    fn png_with_alpha(width: u32, height: u32) -> Vec<u8> {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                // A diagonal transparency ramp so the image has real alpha.
                rgba.extend_from_slice(&[0x40, 0x80, 0xC0, ((x + y) * 8) as u8]);
            }
        }
        let buffer = image::RgbaImage::from_raw(width, height, rgba).unwrap();
        let mut out = std::io::Cursor::new(Vec::new());
        buffer.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn an_image_with_alpha_produces_an_rgba_etc2_sidecar() {
        let bytes = png_with_alpha(16, 16);
        let sidecar = transcode_image("hero.png", &bytes, no_astc()).expect("transcodes");
        let parsed = parse_ktx2(&sidecar.bytes).expect("parses");
        assert_eq!(parsed.header.format, VkFormat::Etc2R8G8B8A8UnormBlock);
        // RGBA is 16 bytes/block, twice the RGB size.
        assert_eq!(parsed.data.len(), (16 / 4) * (16 / 4) * 16);
    }

    #[test]
    fn an_opaque_image_stays_on_the_smaller_rgb_format() {
        // `png()` writes alpha 0xFF everywhere, so despite being an RGBA PNG it
        // must transcode to the half-size RGB format.
        let bytes = png(16, 16);
        let sidecar = transcode_image("bg.png", &bytes, no_astc()).expect("transcodes");
        let parsed = parse_ktx2(&sidecar.bytes).expect("parses");
        assert_eq!(parsed.header.format, VkFormat::Etc2R8G8B8UnormBlock);
        assert_eq!(parsed.data.len(), (16 / 4) * (16 / 4) * 8);
    }

    #[test]
    fn a_non_aligned_image_is_left_to_the_original() {
        // 14 is not a multiple of 4: ETC2 cannot encode it without padding, so
        // no sidecar is produced and the original PNG carries the asset.
        let bytes = png(14, 16);
        assert!(transcode_image("sprite.png", &bytes, no_astc()).is_none());
    }

    #[test]
    fn an_image_with_alpha_takes_astc_where_the_device_decodes_it() {
        let bytes = png_with_alpha(16, 16);
        let sidecar = transcode_image("hero.png", &bytes, with_astc()).expect("transcodes");
        let parsed = parse_ktx2(&sidecar.bytes).expect("parses");
        assert_eq!(parsed.header.format, VkFormat::Astc4x4UnormBlock);
        // The same sixteen bytes per block ETC2 RGBA costs. The format is
        // chosen for what it reconstructs, not for what it saves: an encoder
        // that made this bigger would be a package-size regression for the
        // devices that can read it.
        assert_eq!(parsed.data.len(), (16 / 4) * (16 / 4) * 16);
    }

    #[test]
    fn an_opaque_image_stays_on_etc2_even_where_astc_is_available() {
        // ETC2 RGB is half a byte per pixel; ASTC 4x4 is one. Choosing ASTC
        // here would double the bytes for a format that reconstructs no better,
        // so the device's capability must not decide this one.
        let bytes = png(16, 16);
        let sidecar = transcode_image("bg.png", &bytes, with_astc()).expect("transcodes");
        let parsed = parse_ktx2(&sidecar.bytes).expect("parses");
        assert_eq!(parsed.header.format, VkFormat::Etc2R8G8B8UnormBlock);
        assert_eq!(parsed.data.len(), (16 / 4) * (16 / 4) * 8);
    }

    #[test]
    fn unknown_capabilities_fall_to_the_format_every_device_decodes() {
        // `GpuCapsSnapshot::default()` is what a caller reads before the GPU has
        // reported, and an unknown answer has to land on the universal format
        // rather than the better one.
        let bytes = png_with_alpha(16, 16);
        let sidecar =
            transcode_image("hero.png", &bytes, GpuCapsSnapshot::default()).expect("transcodes");
        let parsed = parse_ktx2(&sidecar.bytes).expect("parses");
        assert_eq!(parsed.header.format, VkFormat::Etc2R8G8B8A8UnormBlock);
    }

    #[test]
    fn non_image_bytes_yield_no_sidecar() {
        assert!(transcode_image("data.png", b"this is not a PNG", no_astc()).is_none());
    }
}
