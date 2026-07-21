//! Persistent derived texture cache.
//!
//! Caches decoded RGBA or compressed texture data on disk per-game.
//! Keyed by `(asset_path, source_generation, gpu_format, target_size)`.
//! Survives across sessions — second load skips IO decode entirely.
//!
//! Storage: `{game_cache_dir}/derived/{sha256_hex}.bin`
//! Each file embeds the full key metadata in the header for verification.
//!
//! # Directory-level quota (P2)
//!
//! The single-file cap (`MAX_DERIVED_FILE_SIZE`) bounds one entry, but
//! without a directory-level quota the cache grows without bound
//! across sessions. [`prune_derived_cache`] enforces a total-bytes
//! budget by evicting least-recently-*accessed* files first. `atime`
//! is approximated by touching `mtime` on every successful
//! [`load_derived`]; this is portable (no `utimensat` dependency) and
//! close enough for LRU purposes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

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

pub fn derived_cache_dir(game_cache_dir: &Path) -> PathBuf {
    game_cache_dir.join("derived")
}

/// Maximum derived cache file size we'll read: 256 MB.
/// Prevents a poisoned/oversized file from consuming all memory.
const MAX_DERIVED_FILE_SIZE: u64 = 256 * 1024 * 1024;

/// Files larger than this threshold are read via mmap instead of
/// `std::fs::read`.  Rationale: the tail of the distribution
/// (high-res textures, pre-composited atlases) pays two full
/// payload copies on the old path — one for the disk buffer and
/// one for `Arc<Vec<u8>>` — and the first is avoidable with mmap.
/// For small entries the mmap syscall pair is pure overhead, so
/// we stay on `fs::read` up to 1 MiB.
const MMAP_THRESHOLD_BYTES: u64 = 1024 * 1024;

/// Default directory-wide budget for derived cache. Chosen so the
/// cache comfortably holds a full session's warm textures but never
/// crowds out user data / subpackage storage on a 8 GiB device.
pub const DEFAULT_DERIVED_CACHE_MAX_BYTES: u64 = 256 * 1024 * 1024;

/// Per-pass soft cap on how many files [`prune_derived_cache`] may
/// inspect. The cache rarely grows past a few thousand files; capping
/// the scan prevents a pathological directory (e.g. corruption) from
/// blocking a session-start prune for minutes.
const PRUNE_MAX_FILES_SCANNED: usize = 32 * 1024;

pub fn load_derived(game_cache_dir: &Path, key: &DerivedKey) -> Option<DecodedImage> {
    let dir = derived_cache_dir(game_cache_dir);
    let path = dir.join(format!("{}.bin", key.hash()));

    // Check file size BEFORE reading to prevent large-file poisoning.
    let meta = std::fs::metadata(&path).ok()?;
    let size = meta.len();
    if size > MAX_DERIVED_FILE_SIZE {
        let _ = std::fs::remove_file(&path);
        return None;
    }

    // Two read paths — both produce `&[u8]` for the shared parser.
    // The mmap path avoids the intermediate `Vec<u8>` the kernel
    // would otherwise copy into; the payload copy inside
    // `parse_derived_entry` still happens but costs one allocation
    // instead of two at the peak.
    let parsed = if size > MMAP_THRESHOLD_BYTES {
        let mapped = crate::mmap_reader::mmap_file_bytes(&path).ok()?;
        parse_derived_entry(mapped.as_slice(), key)
    } else {
        let data = std::fs::read(&path).ok()?;
        parse_derived_entry(&data, key)
    };
    match parsed {
        Some(img) => {
            // Approximate LRU: successful reads bump mtime so the
            // next prune pass keeps hot entries. We rely on mtime
            // because Android's noatime-mounted cache dir never
            // updates atime, so `utimensat(..)` with UTIME_NOW on
            // the mtime half is the only portable signal.
            touch_file_mtime(&path);
            Some(img)
        }
        None => {
            let _ = std::fs::remove_file(&path);
            None
        }
    }
}

fn touch_file_mtime(path: &Path) {
    // Opening O_RDWR would break in a read-only mount; `File::open`
    // then `set_len(len)` with `seek=0` is a portable no-op write
    // that bumps mtime on any Unix filesystem Android cares about.
    // Swallow all failures — mtime touch is a hint, not correctness.
    use std::fs::OpenOptions;
    if let Ok(f) = OpenOptions::new().append(true).open(path) {
        // Re-opening for append + immediately dropping is enough on
        // Linux to bump ctime, but we need mtime — so we explicitly
        // call `set_len` to the existing length, which is a write of
        // zero bytes and always updates mtime.
        if let Ok(meta) = f.metadata() {
            let _ = f.set_len(meta.len());
        }
    }
}

pub fn save_derived(game_cache_dir: &Path, key: &DerivedKey, image: &DecodedImage) {
    let dir = derived_cache_dir(game_cache_dir);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let final_path = dir.join(format!("{}.bin", key.hash()));

    let mut buf = Vec::with_capacity(256);
    if serialize_derived_entry(&mut buf, key, image).is_err() {
        return;
    }

    // Crash-safe: tmp file → write_all → sync_all → rename → dir fsync.
    // Readers never see a half-written file, and a power loss between
    // the rename and the next game session still leaves a valid entry
    // on disk.  Failures are swallowed — the derived cache is an
    // optimisation, not a correctness dependency.
    let _ = crate::atomic_write::atomic_write(&final_path, &buf);
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
        DecodedImage::HardwareBuffer(_) => {
            // AHB-backed images are GPU resources owned by the
            // graphics process; they have no portable on-disk
            // representation. Skip the derived-cache write — the
            // image is already "warm" in the sense that the next
            // load can re-decode straight into a new AHB.
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "derived_cache: HardwareBuffer variant not persistable",
            ));
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

/// Summary of a prune pass for logging / tests.
#[derive(Debug, Default, Clone, Copy)]
pub struct PruneReport {
    pub files_kept: usize,
    pub files_removed: usize,
    pub bytes_kept: u64,
    pub bytes_removed: u64,
    pub scan_capped: bool,
}

/// Enforce a directory-wide byte budget on the derived cache by
/// evicting least-recently-modified files (proxying LRU as explained
/// in the module-level doc) until the total is under `budget_bytes`.
///
/// Safe to call repeatedly: it re-reads the directory each time. The
/// runtime is expected to invoke this at session-start with
/// [`DEFAULT_DERIVED_CACHE_MAX_BYTES`] as a background task.
///
/// Returns a [`PruneReport`] suitable for structured logging.
pub fn prune_derived_cache(game_cache_dir: &Path, budget_bytes: u64) -> PruneReport {
    let dir = derived_cache_dir(game_cache_dir);
    let read_dir = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(_) => return PruneReport::default(), // dir doesn't exist yet
    };

    // Collect (mtime, size, path). Any entry whose metadata we can't
    // read is skipped rather than deleted — that's a transient FS
    // hiccup, not a cache poison.
    let mut scanned = 0usize;
    let mut scan_capped = false;
    let mut entries: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for entry in read_dir.flatten() {
        scanned += 1;
        if scanned > PRUNE_MAX_FILES_SCANNED {
            scan_capped = true;
            break;
        }
        let path = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_file() {
            continue;
        }
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        entries.push((mtime, meta.len(), path));
    }

    let total: u64 = entries.iter().map(|(_, sz, _)| *sz).sum();
    if total <= budget_bytes {
        return PruneReport {
            files_kept: entries.len(),
            files_removed: 0,
            bytes_kept: total,
            bytes_removed: 0,
            scan_capped,
        };
    }

    // Sort oldest-first so the LRU tail is evicted first.
    entries.sort_by_key(|(mtime, _, _)| *mtime);

    let mut running = total;
    let mut bytes_removed = 0u64;
    let mut files_removed = 0usize;
    for (_, sz, path) in &entries {
        if running <= budget_bytes {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            running = running.saturating_sub(*sz);
            bytes_removed = bytes_removed.saturating_add(*sz);
            files_removed += 1;
        }
    }

    PruneReport {
        files_kept: entries.len().saturating_sub(files_removed),
        files_removed,
        bytes_kept: running,
        bytes_removed,
        scan_capped,
    }
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

    // ---- mmap read path (P3-1) --------------------------------

    #[test]
    fn roundtrip_rgba_above_mmap_threshold_uses_mmap_path() {
        // A payload just past the 1 MiB threshold forces the
        // mmap branch in `load_derived`.  The byte-for-byte
        // equality of the RGBA payload proves the mmap slice
        // handed to `parse_derived_entry` matches what
        // `std::fs::read` would have produced — i.e. the
        // two paths are observationally identical.
        let dir = tmp("rt_rgba_mmap");
        let key = DerivedKey {
            asset_path: "big.png".into(),
            source_generation: 7,
            gpu_format: 0,
            variant_kind: 0,
            target_width: 0,
            target_height: 0,
        };
        // 520x520x4 = 1.06 MiB > MMAP_THRESHOLD_BYTES (1 MiB).
        let w = 520u32;
        let h = 520u32;
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        // Non-trivial pattern so a wrong-slice bug would surface.
        for (i, b) in rgba.iter_mut().enumerate() {
            *b = (i & 0xff) as u8;
        }
        let img = DecodedImage::Rgba(NormalizedImage::new(w, h, rgba.clone()));
        save_derived(&dir, &key, &img);

        let loaded = load_derived(&dir, &key).expect("hit via mmap path");
        match loaded {
            DecodedImage::Rgba(n) => {
                assert_eq!(n.width, w);
                assert_eq!(n.height, h);
                assert_eq!(&*n.rgba, &rgba);
            }
            _ => panic!("expected Rgba variant"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mmap_path_rejects_corrupted_large_file_and_cleans_up() {
        // Same corruption scenario as `bad_rgba_length_rejected`
        // but over the mmap threshold.  Both paths must reject
        // invalid payloads and delete the poisoned file.
        let dir = tmp("mmap_corrupt");
        let key = DerivedKey {
            asset_path: "big.png".into(),
            source_generation: 9,
            gpu_format: 0,
            variant_kind: 0,
            target_width: 0,
            target_height: 0,
        };
        let w = 600u32;
        let h = 600u32;
        let rgba = vec![0xCCu8; (w * h * 4) as usize];
        let img = DecodedImage::Rgba(NormalizedImage::new(w, h, rgba));
        save_derived(&dir, &key, &img);

        let path = derived_cache_dir(&dir).join(format!("{}.bin", key.hash()));
        assert!(
            std::fs::metadata(&path).unwrap().len() > MMAP_THRESHOLD_BYTES,
            "test setup must put file over mmap threshold"
        );
        // Truncate to invalidate the embedded payload length.
        let mut data = std::fs::read(&path).unwrap();
        data.truncate(data.len() - 100);
        std::fs::write(&path, &data).unwrap();

        assert!(load_derived(&dir, &key).is_none());
        assert!(!path.exists(), "poisoned file must be cleaned up");

        let _ = std::fs::remove_dir_all(&dir);
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

        // Final file exists, no .tmp / .atomic.tmp leftover.
        let final_path = derived_cache_dir(&dir).join(format!("{}.bin", key.hash()));
        assert!(final_path.exists());
        let leftovers: Vec<_> = std::fs::read_dir(derived_cache_dir(&dir))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| {
                name.starts_with(&format!("{}.tmp", key.hash())) || name.ends_with(".atomic.tmp")
            })
            .collect();
        assert!(leftovers.is_empty(), "stale tmp: {:?}", leftovers);

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
    fn prune_respects_budget_and_preserves_newer_files() {
        let dir = tmp("prune_budget");
        let cache_dir = derived_cache_dir(&dir);
        std::fs::create_dir_all(&cache_dir).unwrap();
        // Write files with a short sleep between them so each entry
        // has a strictly greater mtime than the previous. Sleep is
        // 20 ms which is well above every filesystem's mtime
        // resolution the runtime will ship on (ext4, f2fs, tmpfs).
        for i in 0..6u32 {
            let path = cache_dir.join(format!("f_{i:02}.bin"));
            std::fs::write(&path, vec![0u8; 1024]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let report = prune_derived_cache(&dir, 3 * 1024);
        assert!(report.bytes_kept <= 3 * 1024);
        assert!(report.files_removed >= 3);
        // Oldest files removed first.
        assert!(!cache_dir.join("f_00.bin").exists());
        assert!(cache_dir.join("f_05.bin").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_noop_when_under_budget() {
        let dir = tmp("prune_noop");
        let cache_dir = derived_cache_dir(&dir);
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::write(cache_dir.join("a.bin"), vec![0u8; 128]).unwrap();
        let report = prune_derived_cache(&dir, 1024);
        assert_eq!(report.files_removed, 0);
        assert!(cache_dir.join("a.bin").exists());
        let _ = std::fs::remove_dir_all(&dir);
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
