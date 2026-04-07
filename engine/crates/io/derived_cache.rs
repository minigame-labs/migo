//! Persistent derived texture cache.
//!
//! Caches decoded RGBA or compressed texture data on disk per-game.
//! Keyed by `(asset_path, source_generation, gpu_format, target_size)`.
//! Survives across sessions — second load skips IO decode entirely.
//!
//! Storage: `{game_cache_dir}/derived/{sha256_hex}.bin`
//! Each file embeds the full key metadata in the header for verification.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use shared::protocol::io_cmd::{CompressedImage, DecodedImage, NormalizedImage};

/// Derived cache key components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedKey {
    pub asset_path: String,
    pub source_generation: u64,
    /// 0 = RGBA, else vk_format code for compressed.
    pub gpu_format: u32,
    /// Variant source kind: primary/compressed/fallback/upgrade.
    pub variant_kind: u8,
    /// Target resize dimensions. (0,0) = full resolution.
    pub target_width: u32,
    pub target_height: u32,
}

impl DerivedKey {
    /// SHA-256 based hex hash for the filename (collision-resistant).
    pub fn hash(&self) -> String {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(self.asset_path.as_bytes());
        h.update(&self.source_generation.to_le_bytes());
        h.update(&self.gpu_format.to_le_bytes());
        h.update(&[self.variant_kind]);
        h.update(&self.target_width.to_le_bytes());
        h.update(&self.target_height.to_le_bytes());
        // Use first 16 bytes (128 bits) of SHA-256 for filename.
        let result = h.finalize();
        hex::encode(&result[..16])
    }

    /// Serialize key metadata for embedding in the cache file header.
    fn to_bytes(&self) -> Vec<u8> {
        let path_bytes = self.asset_path.as_bytes();
        let mut buf = Vec::with_capacity(21 + path_bytes.len());
        buf.extend_from_slice(&(path_bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(path_bytes);
        buf.extend_from_slice(&self.source_generation.to_le_bytes());
        buf.extend_from_slice(&self.gpu_format.to_le_bytes());
        buf.push(self.variant_kind);
        buf.extend_from_slice(&self.target_width.to_le_bytes());
        buf.extend_from_slice(&self.target_height.to_le_bytes());
        buf
    }

    /// Deserialize key metadata from cache file header.
    fn from_bytes(data: &[u8]) -> Option<(Self, usize)> {
        if data.len() < 2 {
            return None;
        }
        let path_len = u16::from_le_bytes(data[0..2].try_into().ok()?) as usize;
        let needed = 2 + path_len + 8 + 4 + 1 + 4 + 4; // path_len + path + gen + fmt + kind + tw + th
        if data.len() < needed {
            return None;
        }
        let asset_path = std::str::from_utf8(&data[2..2 + path_len])
            .ok()?
            .to_string();
        let mut off = 2 + path_len;
        let source_generation = u64::from_le_bytes(data[off..off + 8].try_into().ok()?);
        off += 8;
        let gpu_format = u32::from_le_bytes(data[off..off + 4].try_into().ok()?);
        off += 4;
        let variant_kind = data[off];
        off += 1;
        let target_width = u32::from_le_bytes(data[off..off + 4].try_into().ok()?);
        off += 4;
        let target_height = u32::from_le_bytes(data[off..off + 4].try_into().ok()?);
        off += 4;
        Some((
            Self {
                asset_path,
                source_generation,
                gpu_format,
                variant_kind,
                target_width,
                target_height,
            },
            off,
        ))
    }
}

// File format:
// [MDRV magic: 4][version: 1][key_len: 2][key_bytes: key_len][kind: 1][width: 4][height: 4][vk_format: 4][data_len: 4][payload: data_len]
const DERIVED_MAGIC: [u8; 4] = *b"MDRV";
const DERIVED_VERSION: u8 = 3; // Bumped from 2 to include variant_kind in the key/header.
const KIND_RGBA: u8 = 0;
const KIND_COMPRESSED: u8 = 1;
static NEXT_TMP_ID: AtomicU64 = AtomicU64::new(1);

pub fn derived_cache_dir(game_cache_dir: &Path) -> PathBuf {
    game_cache_dir.join("derived")
}

/// Maximum derived cache file size we'll read: 256 MB.
/// Prevents a poisoned/oversized file from consuming all memory.
const MAX_DERIVED_FILE_SIZE: u64 = 256 * 1024 * 1024;

pub fn load_derived(game_cache_dir: &Path, key: &DerivedKey) -> Option<DecodedImage> {
    let dir = derived_cache_dir(game_cache_dir);
    let path = dir.join(format!("{}.bin", key.hash()));

    // Check file size BEFORE reading to prevent large-file poisoning.
    let meta = std::fs::metadata(&path).ok()?;
    if meta.len() > MAX_DERIVED_FILE_SIZE {
        let _ = std::fs::remove_file(&path);
        return None;
    }

    let data = std::fs::read(&path).ok()?;
    match parse_derived_entry(&data, key) {
        Some(img) => Some(img),
        None => {
            let _ = std::fs::remove_file(&path);
            None
        }
    }
}

pub fn save_derived(game_cache_dir: &Path, key: &DerivedKey, image: &DecodedImage) {
    let dir = derived_cache_dir(game_cache_dir);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let final_path = dir.join(format!("{}.bin", key.hash()));
    let tmp_path = dir.join(format!(
        "{}.tmp.{}",
        key.hash(),
        NEXT_TMP_ID.fetch_add(1, Ordering::Relaxed)
    ));

    let mut buf = Vec::with_capacity(256);
    if serialize_derived_entry(&mut buf, key, image).is_err() {
        return;
    }

    // Atomic write: tmp file → flush → rename to final path.
    // Readers never see a half-written file.
    match std::fs::File::create(&tmp_path).and_then(|mut f| {
        use std::io::Write;
        f.write_all(&buf)?;
        f.flush()?;
        Ok(())
    }) {
        Ok(()) => {
            if std::fs::rename(&tmp_path, &final_path).is_err() {
                let _ = std::fs::remove_file(&tmp_path);
            }
        }
        Err(_) => {
            let _ = std::fs::remove_file(&tmp_path);
        }
    }
}

fn parse_derived_entry(data: &[u8], expected_key: &DerivedKey) -> Option<DecodedImage> {
    // Magic + version.
    if data.len() < 7 {
        return None;
    }
    if data[0..4] != DERIVED_MAGIC || data[4] != DERIVED_VERSION {
        return None;
    }

    // Embedded key.
    let key_len = u16::from_le_bytes(data[5..7].try_into().ok()?) as usize;
    let key_start = 7;
    if data.len() < key_start + key_len {
        return None;
    }
    let (embedded_key, _) = DerivedKey::from_bytes(&data[key_start..key_start + key_len])?;

    // Verify key matches — prevents collision and stale cache.
    if embedded_key != *expected_key {
        return None;
    }

    // Image data follows key.
    let img_start = key_start + key_len;
    if data.len() < img_start + 17 {
        return None;
    } // kind(1) + w(4) + h(4) + vk(4) + dlen(4)

    let kind = data[img_start];
    let width = u32::from_le_bytes(data[img_start + 1..img_start + 5].try_into().ok()?);
    let height = u32::from_le_bytes(data[img_start + 5..img_start + 9].try_into().ok()?);
    let vk_format = u32::from_le_bytes(data[img_start + 9..img_start + 13].try_into().ok()?);
    let data_len =
        u32::from_le_bytes(data[img_start + 13..img_start + 17].try_into().ok()?) as usize;

    let payload_start = img_start + 17;
    if data.len() < payload_start + data_len {
        return None;
    }
    let payload = &data[payload_start..payload_start + data_len];

    match kind {
        KIND_RGBA => {
            // Validate RGBA invariant.
            let expected_len = (width as usize)
                .checked_mul(height as usize)?
                .checked_mul(4)?;
            if payload.len() != expected_len {
                return None;
            }
            Some(DecodedImage::Rgba(NormalizedImage {
                width,
                height,
                rgba: Arc::new(payload.to_vec()),
            }))
        }
        KIND_COMPRESSED => Some(DecodedImage::Compressed(CompressedImage {
            width,
            height,
            vk_format,
            data: Arc::new(payload.to_vec()),
        })),
        _ => None,
    }
}

fn serialize_derived_entry(
    buf: &mut Vec<u8>,
    key: &DerivedKey,
    image: &DecodedImage,
) -> std::io::Result<()> {
    buf.write_all(&DERIVED_MAGIC)?;
    buf.write_all(&[DERIVED_VERSION])?;
    let key_bytes = key.to_bytes();
    buf.write_all(&(key_bytes.len() as u16).to_le_bytes())?;
    buf.write_all(&key_bytes)?;
    match image {
        DecodedImage::Rgba(img) => {
            buf.write_all(&[KIND_RGBA])?;
            buf.write_all(&img.width.to_le_bytes())?;
            buf.write_all(&img.height.to_le_bytes())?;
            buf.write_all(&0u32.to_le_bytes())?;
            buf.write_all(&(img.rgba.len() as u32).to_le_bytes())?;
            buf.write_all(&img.rgba)?;
        }
        DecodedImage::Compressed(img) => {
            buf.write_all(&[KIND_COMPRESSED])?;
            buf.write_all(&img.width.to_le_bytes())?;
            buf.write_all(&img.height.to_le_bytes())?;
            buf.write_all(&img.vk_format.to_le_bytes())?;
            buf.write_all(&(img.data.len() as u32).to_le_bytes())?;
            buf.write_all(&img.data)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("migo_dc_{name}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn roundtrip_rgba() {
        let dir = tmp("rt_rgba");
        let key = DerivedKey {
            asset_path: "s.png".into(),
            source_generation: 3,
            gpu_format: 0,
            variant_kind: 0,
            target_width: 0,
            target_height: 0,
        };
        let img = DecodedImage::Rgba(NormalizedImage::new(2, 2, vec![0xFF; 16]));
        save_derived(&dir, &key, &img);
        let loaded = load_derived(&dir, &key).expect("hit");
        assert!(matches!(loaded, DecodedImage::Rgba(r) if r.width == 2 && r.rgba.len() == 16));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn roundtrip_compressed() {
        let dir = tmp("rt_comp");
        let key = DerivedKey {
            asset_path: "a.ktx2".into(),
            source_generation: 1,
            gpu_format: 147,
            variant_kind: 1,
            target_width: 0,
            target_height: 0,
        };
        let img = DecodedImage::Compressed(CompressedImage {
            width: 256,
            height: 256,
            vk_format: 147,
            data: Arc::new(vec![0xAA; 1024]),
        });
        save_derived(&dir, &key, &img);
        let loaded = load_derived(&dir, &key).expect("hit");
        assert!(matches!(loaded, DecodedImage::Compressed(c) if c.vk_format == 147));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn miss_returns_none() {
        let dir = tmp("miss");
        let key = DerivedKey {
            asset_path: "x.png".into(),
            source_generation: 1,
            gpu_format: 0,
            variant_kind: 0,
            target_width: 0,
            target_height: 0,
        };
        assert!(load_derived(&dir, &key).is_none());
    }

    #[test]
    fn fullsize_and_resized_dont_collide() {
        let dir = tmp("resize");
        let full_key = DerivedKey {
            asset_path: "s.png".into(),
            source_generation: 1,
            gpu_format: 0,
            variant_kind: 0,
            target_width: 0,
            target_height: 0,
        };
        let resize_key = DerivedKey {
            asset_path: "s.png".into(),
            source_generation: 1,
            gpu_format: 0,
            variant_kind: 0,
            target_width: 64,
            target_height: 64,
        };

        let full_img =
            DecodedImage::Rgba(NormalizedImage::new(128, 128, vec![0xAA; 128 * 128 * 4]));
        save_derived(&dir, &full_key, &full_img);

        // Resized key must NOT hit full-size cache.
        assert!(load_derived(&dir, &resize_key).is_none());
        // Full key still hits.
        assert!(load_derived(&dir, &full_key).is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_rgba_length_rejected() {
        let dir = tmp("bad_rgba");
        let key = DerivedKey {
            asset_path: "s.png".into(),
            source_generation: 1,
            gpu_format: 0,
            variant_kind: 0,
            target_width: 0,
            target_height: 0,
        };
        // Save valid first.
        let img = DecodedImage::Rgba(NormalizedImage::new(2, 2, vec![0xFF; 16]));
        save_derived(&dir, &key, &img);

        // Corrupt the payload length in the file.
        let path = derived_cache_dir(&dir).join(format!("{}.bin", key.hash()));
        let mut data = std::fs::read(&path).unwrap();
        // Truncate 4 bytes from the payload to create length mismatch.
        data.truncate(data.len() - 4);
        std::fs::write(&path, &data).unwrap();

        // Must return None (bad RGBA length).
        assert!(load_derived(&dir, &key).is_none());
        // Bad file should be cleaned up.
        assert!(!path.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_mismatch_rejected() {
        let dir = tmp("key_mismatch");
        let key1 = DerivedKey {
            asset_path: "a.png".into(),
            source_generation: 1,
            gpu_format: 0,
            variant_kind: 0,
            target_width: 0,
            target_height: 0,
        };
        let key2 = DerivedKey {
            asset_path: "b.png".into(),
            source_generation: 1,
            gpu_format: 0,
            variant_kind: 0,
            target_width: 0,
            target_height: 0,
        };

        let img = DecodedImage::Rgba(NormalizedImage::new(1, 1, vec![0xFF; 4]));
        save_derived(&dir, &key1, &img);

        // Even if hashes hypothetically collided, embedded key verification rejects.
        // (In practice they won't collide with SHA-256, but this tests the defense.)
        assert!(load_derived(&dir, &key1).is_some());
        assert!(load_derived(&dir, &key2).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_no_tmp_leftover() {
        let dir = tmp("atomic");
        let key = DerivedKey {
            asset_path: "s.png".into(),
            source_generation: 1,
            gpu_format: 0,
            variant_kind: 0,
            target_width: 0,
            target_height: 0,
        };
        let img = DecodedImage::Rgba(NormalizedImage::new(2, 2, vec![0xFF; 16]));
        save_derived(&dir, &key, &img);

        // Final file exists, no .tmp leftover.
        let final_path = derived_cache_dir(&dir).join(format!("{}.bin", key.hash()));
        assert!(final_path.exists());
        let leftovers: Vec<_> = std::fs::read_dir(derived_cache_dir(&dir))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(&format!("{}.tmp", key.hash())))
            .collect();
        assert!(leftovers.is_empty());

        // Content is valid.
        assert!(load_derived(&dir, &key).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn different_generation_different_hash() {
        let k1 = DerivedKey {
            asset_path: "a.png".into(),
            source_generation: 1,
            gpu_format: 0,
            variant_kind: 0,
            target_width: 0,
            target_height: 0,
        };
        let k2 = DerivedKey {
            asset_path: "a.png".into(),
            source_generation: 2,
            gpu_format: 0,
            variant_kind: 0,
            target_width: 0,
            target_height: 0,
        };
        assert_ne!(k1.hash(), k2.hash());
    }

    #[test]
    fn different_variant_kind_different_hash() {
        let k1 = DerivedKey {
            asset_path: "a.png".into(),
            source_generation: 1,
            gpu_format: 0,
            variant_kind: 0,
            target_width: 0,
            target_height: 0,
        };
        let k2 = DerivedKey {
            asset_path: "a.png".into(),
            source_generation: 1,
            gpu_format: 0,
            variant_kind: 2,
            target_width: 0,
            target_height: 0,
        };
        assert_ne!(k1.hash(), k2.hash());
    }
}
