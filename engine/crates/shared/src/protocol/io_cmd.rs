use std::sync::Arc;

pub type FileId = u32;

/// Maximum bytes a single read op may request (100 MiB).
/// Matches the platform API limit ("single file size up to 100M").
pub const MAX_READ_LENGTH: u64 = 100 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenFlag {
    Read,
    ReadWrite,
    WriteTruncateCreate,
    ReadWriteTruncateCreate,
    AppendCreate,
    ReadAppendCreate,
    /// 'ax' – append, fail if path already exists
    AppendExclusive,
    /// 'ax+' – read+append, fail if path already exists
    ReadAppendExclusive,
    /// 'as' – append (sync I/O hint)
    AppendSyncCreate,
    /// 'as+' – read+append (sync I/O hint)
    ReadAppendSyncCreate,
    /// Write-exclusive mode – write+truncate, fail if path already exists
    WriteExclusive,
    /// Read/write-exclusive mode – read+write+truncate, fail if path already exists
    ReadWriteExclusive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    /// Overwrite the file (create if missing).
    Overwrite,
    /// Append to the file (create if missing).
    Append,
}

/// Crash-durability level for writes.
///
/// The default is [`Durable`](WriteDurability::Durable): the overwrite path
/// uses the atomic `temp -> fsync -> rename -> dir fsync` sequence and the
/// append path `fsync`s, so a crash / power loss never leaves a torn or
/// lost write. This is the correct default for game saves.
///
/// [`Fast`](WriteDurability::Fast) trades that guarantee for throughput:
/// overwrite does a plain truncating write and append skips the per-write
/// `fsync`. Suitable for high-frequency scratch / cache writes the game can
/// afford to lose on a crash. Callers opt in explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WriteDurability {
    #[default]
    Durable,
    Fast,
}

/// Image cache statistics
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImageCacheStats {
    /// Number of cached entries
    pub entries: usize,
    /// Current size in bytes
    pub size_bytes: usize,
    /// Maximum size in bytes
    pub max_bytes: usize,
    /// Cache hits
    pub hits: u64,
    /// Cache misses
    pub misses: u64,
    /// Hit rate (0.0 to 1.0)
    pub hit_rate: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileStat {
    pub mode: u32,
    pub size: u64,
    pub atime: u64,
    pub mtime: u64,
    pub is_file: bool,
    pub is_directory: bool,
}

/// A single entry in the recursive stat result.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StatEntry {
    pub path: String,
    pub stat: FileStat,
}

/// Stat result — typed union for single-file vs recursive responses.
/// `#[serde(untagged)]` ensures `Single` serializes as a flat object and
/// `Recursive` serializes as an array, matching the existing JS expectations.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum StatResult {
    Single(FileStat),
    Recursive(Vec<StatEntry>),
}

/// Payload for one zip entry.
///
/// Binary entries stay bytes all the way to the `ArrayBuffer` the caller
/// receives. They used to be base64-encoded for transport, which cost 1.33x
/// inflation on the wire plus a per-byte decode loop in JS.
#[derive(Debug)]
pub enum ZipEntryData {
    /// The entry decoded with the caller's requested encoding.
    Text(String),
    /// Raw bytes, for a caller that asked for no encoding.
    Binary(Vec<u8>),
}

/// Result for a single entry read from a zip archive.
#[derive(Debug)]
pub struct ZipEntryResult {
    pub path: String,
    /// `None` when the entry could not be read; `err_msg` says why.
    pub data: Option<ZipEntryData>,
    pub err_msg: String,
}

/// Info about a saved file, returned by ListSavedFiles.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SavedFileInfo {
    #[serde(rename = "filePath")]
    pub file_path: String,
    pub size: u64,
    #[serde(rename = "createTime")]
    pub create_time: u64,
}

/// Hard upper bound on decoded image dimensions (total pixel count), shared as
/// the single source of truth across the decode layer (`io`) and the GPU upload
/// layer (`graphics`) so the two can never drift apart.
///
/// A tiny compressed file can declare enormous dimensions — e.g. a ~50 KB PNG
/// with a `30000x30000` header decodes to 3.6 GiB of RGBA, and a small KTX2 can
/// declare an equally huge GPU texture. 64 Mpx = `8192x8192` = 256 MiB RGBA is
/// generous for legitimate high-res bundled game textures (8K square) while
/// blocking the multi-GiB allocations that instantly OOM the process. The
/// inline/HTTP paths keep their own tighter budget for untrusted sources.
///
/// Mirrored in the Android platform decoder (`NativeExports.decodeImageRgba`)
/// as an early-rejection optimisation — keep the two in sync when changing this
/// value. The Rust `enforce_pixel_cap` is authoritative: it re-checks the
/// platform-decoder result, so a Java-side drift only wastes one bitmap decode,
/// it cannot defeat the OOM guard.
pub const MAX_IMAGE_PIXELS: u64 = 8192 * 8192;

/// A simple, protocol-friendly image representation.
/// - `rgba` length must be width * height * 4.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NormalizedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

impl NormalizedImage {
    #[inline]
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        Self {
            width,
            height,
            rgba: Arc::new(rgba),
        }
    }
}

/// GPU-ready compressed texture data (e.g., ETC2/ASTC from KTX2).
/// Bypasses RGBA decode entirely — uploaded via `glCompressedTexImage2D`.
#[derive(Debug, Clone)]
pub struct CompressedImage {
    pub width: u32,
    pub height: u32,
    /// Vulkan format code from the KTX2 header (e.g. 147=ETC2_RGB).
    pub vk_format: u32,
    /// Every mip level's block data, concatenated base level first.
    ///
    /// One allocation rather than one per level: the levels are uploaded in
    /// order and never individually replaced, so splitting them into separate
    /// buffers would cost allocations and cache locality for nothing. Use
    /// [`CompressedImage::levels`] to read them back.
    pub data: Arc<Vec<u8>>,
    /// Byte length of each level within `data`, base level first.
    ///
    /// Never empty, and always sums to `data.len()` -- [`CompressedImage::new`]
    /// is the only way to build one, so the two cannot disagree.
    level_lengths: Vec<u32>,
}

impl CompressedImage {
    /// Build from the levels of a mip chain, base level first.
    ///
    /// Returns `None` for an empty chain or one whose total size does not fit
    /// the counters -- a texture with no base level is not a texture.
    pub fn new(width: u32, height: u32, vk_format: u32, levels: &[&[u8]]) -> Option<Self> {
        if levels.is_empty() {
            return None;
        }
        let mut level_lengths = Vec::with_capacity(levels.len());
        let mut data = Vec::with_capacity(levels.iter().map(|l| l.len()).sum());
        for level in levels {
            level_lengths.push(u32::try_from(level.len()).ok()?);
            data.extend_from_slice(level);
        }
        Some(Self {
            width,
            height,
            vk_format,
            data: Arc::new(data),
            level_lengths,
        })
    }

    /// Rebuild from bytes that were already concatenated, e.g. by a cache.
    ///
    /// Returns `None` unless the lengths account for exactly `data`, which is
    /// what stops a corrupted cache entry from producing levels that overlap or
    /// run past the buffer.
    pub fn from_concatenated(
        width: u32,
        height: u32,
        vk_format: u32,
        data: Arc<Vec<u8>>,
        level_lengths: Vec<u32>,
    ) -> Option<Self> {
        if level_lengths.is_empty() || level_lengths.contains(&0) {
            return None;
        }
        let total: u64 = level_lengths.iter().map(|&len| u64::from(len)).sum();
        if total != data.len() as u64 {
            return None;
        }
        Some(Self {
            width,
            height,
            vk_format,
            data,
            level_lengths,
        })
    }

    /// The mip levels, base level first.
    pub fn levels(&self) -> impl Iterator<Item = &[u8]> + '_ {
        let mut offset = 0usize;
        self.level_lengths.iter().map(move |&len| {
            let start = offset;
            offset += len as usize;
            &self.data[start..offset]
        })
    }

    /// How many mip levels this image carries. Always at least 1.
    pub fn level_count(&self) -> usize {
        self.level_lengths.len()
    }

    /// The per-level byte lengths, base level first.
    pub fn level_lengths(&self) -> &[u32] {
        &self.level_lengths
    }
}

/// Decoded image — one of three on-device representations:
///
/// * **`Rgba`** — host-allocated RGBA8 buffer. Compatible everywhere.
///   Used for legacy/dev paths and as the fall-through for the
///   `getImageData` / `readPixels` round-trip that needs CPU pixels.
/// * **`Compressed`** — GPU-native block-compressed payload (e.g.
///   ETC2 / ASTC unpacked from a KTX2 container). Uploaded directly
///   via `glCompressedTexImage2D`, never decompressed on the CPU.
/// * **`HardwareBuffer`** — Android `AHardwareBuffer`. The decoder
///   wrote pixels straight into a GPU-importable buffer; uploaded
///   via `eglCreateImageKHR(EGL_NATIVE_BUFFER_ANDROID, …)` →
///   `glEGLImageTargetTexture2DOES`, no CPU bytes touched. The
///   refcount is held inside [`crate::protocol::ahb::OwnedAhb`].
///
/// `Clone` is cheap on every variant: `Rgba` clones an `Arc<Vec<u8>>`,
/// `Compressed` clones an `Arc<Vec<u8>>`, `HardwareBuffer` clones an
/// `Arc<AhbBox>` (one Rust atomic refcount increment).
#[derive(Debug, Clone)]
pub enum DecodedImage {
    Rgba(NormalizedImage),
    Compressed(CompressedImage),
    HardwareBuffer(AhbImage),
}

/// AHB-backed decoded image. The buffer width/height come from the
/// AHB descriptor, but they're cached at the wrapper level so
/// downstream code doesn't need to re-`describe` the AHB on every
/// access.
#[derive(Debug, Clone)]
pub struct AhbImage {
    /// Logical image dimensions (≤ stride). Mirror what the JS
    /// `Image.naturalWidth` / `naturalHeight` should report.
    pub width: u32,
    pub height: u32,
    /// Owns the underlying `AHardwareBuffer*` (Android) or the mock
    /// pixel buffer (other targets). Refcounted; `clone` is cheap.
    pub ahb: crate::protocol::ahb::OwnedAhb,
}

impl AhbImage {
    /// Build an `AhbImage`. The caller must ensure the AHB
    /// descriptor's `width`/`height` match the logical image
    /// dimensions; we assert in debug.
    #[inline]
    pub fn new(width: u32, height: u32, ahb: crate::protocol::ahb::OwnedAhb) -> Self {
        debug_assert_eq!(ahb.desc().width, width, "AhbImage width mismatch");
        debug_assert_eq!(ahb.desc().height, height, "AhbImage height mismatch");
        Self { width, height, ahb }
    }
}

impl DecodedImage {
    /// Logical image width across all variants.
    #[inline]
    pub fn width(&self) -> u32 {
        match self {
            DecodedImage::Rgba(r) => r.width,
            DecodedImage::Compressed(c) => c.width,
            DecodedImage::HardwareBuffer(h) => h.width,
        }
    }

    /// Logical image height across all variants.
    #[inline]
    pub fn height(&self) -> u32 {
        match self {
            DecodedImage::Rgba(r) => r.height,
            DecodedImage::Compressed(c) => c.height,
            DecodedImage::HardwareBuffer(h) => h.height,
        }
    }

    /// True for the AHB variant. Hot path predicates that branch on
    /// "do I need to memcpy this through CPU?" use this instead of
    /// pattern-matching to keep the call site short.
    #[inline]
    pub fn is_hardware_buffer(&self) -> bool {
        matches!(self, DecodedImage::HardwareBuffer(_))
    }

    /// Convert this image into a plain RGBA `NormalizedImage` if it
    /// isn't already one. The AHB variant locks the buffer for CPU
    /// read and copies the pixels out — used by `getImageData` /
    /// `readPixels` paths that genuinely need CPU bytes. Compressed
    /// images return `Err(NotImplemented)` because GPU-block decode
    /// belongs to the renderer, not this protocol layer.
    pub fn into_rgba(self) -> Result<NormalizedImage, crate::error::EngineError> {
        match self {
            DecodedImage::Rgba(r) => Ok(r),
            DecodedImage::HardwareBuffer(AhbImage { width, height, ahb }) => {
                let rgba = crate::protocol::ahb::read_rgba_from_ahb(&ahb).map_err(|e| {
                    crate::error::EngineError::new(crate::error::ErrorCode::IoError)
                        .with_msg("DecodedImage::into_rgba: AHB read failed")
                        .with_detail(e.to_string())
                })?;
                Ok(NormalizedImage::new(width, height, rgba))
            }
            DecodedImage::Compressed(_) => Err(crate::error::EngineError::new(
                crate::error::ErrorCode::Unsupported,
            )
            .with_msg("DecodedImage::into_rgba: GPU-compressed source")
            .with_detail("decompress on the renderer or pass through GL_OES_compressed_*")),
        }
    }
}

/// Image load priority for scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImagePriority {
    /// Startup-critical or scene-critical — upload immediately.
    Critical = 0,
    /// Normal interactive use — standard upload path.
    Normal = 1,
    /// Background preload — defer if upload budget is busy.
    Background = 2,
}

impl Default for ImagePriority {
    fn default() -> Self {
        Self::Normal
    }
}

/// Known image variant extensions for companion file lookup and cache keying.
pub const VARIANT_EXTENSIONS: &[&str] = &["ktx2", "png", "jpg", "jpeg", "webp"];

/// Extract the extensionless stem path: `parent/stem` (no trailing extension).
/// E.g. `/data/tex.png` -> `/data/tex`, `/data/tex` -> `/data/tex`.
pub fn path_stem(path: &str) -> String {
    let p = std::path::Path::new(path);
    p.parent()
        .map(|parent| {
            let stem_name = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_else(|| p.file_name().and_then(|s| s.to_str()).unwrap_or(""));
            parent.join(stem_name).to_string_lossy().into_owned()
        })
        .unwrap_or_else(|| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .or_else(|| p.file_name().and_then(|s| s.to_str()))
                .unwrap_or(path)
                .to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ahb::{AhbDesc, OwnedAhb, write_rgba_into_ahb};

    fn checker_2x2() -> Vec<u8> {
        // 2x2 RGBA, alternating opaque red / opaque blue.
        vec![
            255, 0, 0, 255, 0, 0, 255, 255, 0, 0, 255, 255, 255, 0, 0, 255,
        ]
    }

    #[test]
    fn into_rgba_passes_rgba_through_unchanged() {
        let pixels = checker_2x2();
        let img = DecodedImage::Rgba(NormalizedImage::new(2, 2, pixels.clone()));
        let r = img.into_rgba().expect("rgba passthrough");
        assert_eq!(*r.rgba, pixels);
        assert_eq!(r.width, 2);
        assert_eq!(r.height, 2);
    }

    #[test]
    fn into_rgba_downgrades_hardware_buffer_with_identical_pixels() {
        // The downgrade is the M2.6 contract: every byte that the
        // decoder wrote into the AHB must come back out of `into_rgba`
        // unchanged, regardless of the driver's row stride padding.
        let pixels = checker_2x2();
        let ahb = OwnedAhb::allocate(AhbDesc::rgba_sampled_cpu_decode(2, 2)).expect("alloc");
        write_rgba_into_ahb(&ahb, &pixels).expect("write");
        let img = DecodedImage::HardwareBuffer(AhbImage::new(2, 2, ahb));

        let r = img.into_rgba().expect("ahb downgrade");
        assert_eq!(*r.rgba, pixels);
        assert_eq!(r.width, 2);
        assert_eq!(r.height, 2);
    }

    #[test]
    fn levels_round_trip_what_they_were_built_from() {
        let l0 = vec![0xA0; 32];
        let l1 = vec![0xA1; 8];
        let l2 = vec![0xA2; 8];
        let img = CompressedImage::new(8, 8, 147, &[&l0[..], &l1[..], &l2[..]])
            .expect("a three-level chain");

        assert_eq!(img.level_count(), 3);
        let levels: Vec<&[u8]> = img.levels().collect();
        assert_eq!(levels, vec![&l0[..], &l1[..], &l2[..]]);
        assert_eq!(img.data.len(), 48, "levels share one allocation");
    }

    #[test]
    fn a_single_level_image_reports_one_level() {
        let l0 = vec![0xB0; 16];
        let img = CompressedImage::new(4, 4, 147, &[&l0[..]]).expect("one level");

        assert_eq!(img.level_count(), 1);
        assert_eq!(img.levels().next().unwrap(), &l0[..]);
    }

    #[test]
    fn an_empty_chain_is_not_an_image() {
        assert!(CompressedImage::new(4, 4, 147, &[]).is_none());
    }

    #[test]
    fn concatenated_bytes_must_match_the_declared_lengths() {
        let bytes = std::sync::Arc::new(vec![0u8; 40]);
        // A cache entry whose lengths do not account for its payload would
        // otherwise yield levels that overlap or run past the buffer.
        assert!(
            CompressedImage::from_concatenated(8, 8, 147, bytes.clone(), vec![32, 16]).is_none()
        );
        assert!(CompressedImage::from_concatenated(8, 8, 147, bytes.clone(), vec![]).is_none());
        assert!(
            CompressedImage::from_concatenated(8, 8, 147, bytes.clone(), vec![32, 0, 8]).is_none()
        );
        assert!(CompressedImage::from_concatenated(8, 8, 147, bytes, vec![32, 8]).is_some());
    }

    #[test]
    fn into_rgba_rejects_compressed() {
        let img = DecodedImage::Compressed(
            CompressedImage::new(4, 4, 147, &[&[0u8; 32][..]]).expect("one level"),
        );
        let err = img.into_rgba().expect_err("compressed cannot downgrade");
        assert_eq!(err.code, crate::error::ErrorCode::Unsupported);
    }

    #[test]
    fn width_height_consistent_across_variants() {
        // The `width()` / `height()` accessors must report the
        // *logical* image dims, not the driver's padded stride.
        let ahb = OwnedAhb::allocate(AhbDesc::rgba_sampled(7, 5)).unwrap();
        let img = DecodedImage::HardwareBuffer(AhbImage::new(7, 5, ahb));
        assert_eq!(img.width(), 7);
        assert_eq!(img.height(), 5);
        assert!(img.is_hardware_buffer());
    }

    #[test]
    fn cloning_decoded_image_does_not_copy_pixels() {
        let pixels = checker_2x2();
        let ahb = OwnedAhb::allocate(AhbDesc::rgba_sampled_cpu_decode(2, 2)).unwrap();
        write_rgba_into_ahb(&ahb, &pixels).unwrap();
        let img = DecodedImage::HardwareBuffer(AhbImage::new(2, 2, ahb));

        let img2 = img.clone();
        // Both clones see the same logical pixels.
        let a = img.into_rgba().unwrap();
        let b = img2.into_rgba().unwrap();
        assert_eq!(*a.rgba, *b.rgba);
    }
}
