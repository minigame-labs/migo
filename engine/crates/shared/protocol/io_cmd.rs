use std::sync::Arc;

use deno_core::v8;
use tokio::sync::oneshot;

use crate::error::{EngineError, ErrorCode};

pub type FileId = u32;

/// Maximum bytes a single read op may request (100 MiB).
/// Matches the platform API limit ("single file size up to 100M").
pub const MAX_READ_LENGTH: u64 = 100 * 1024 * 1024;

/// Protocol-wide IO result type.
pub type IOResult<T> = Result<T, EngineError>;

#[derive(Debug)]
pub enum IOCmdResp<T> {
    Async(oneshot::Sender<IOResult<T>>),
    Sync(crossbeam_channel::Sender<IOResult<T>>),
}

impl<T> IOCmdResp<T> {
    #[inline]
    pub fn send(self, result: IOResult<T>) {
        match self {
            IOCmdResp::Async(tx) => {
                let _ = tx.send(result);
            }
            IOCmdResp::Sync(tx) => {
                let _ = tx.send(result);
            }
        }
    }

    #[inline]
    pub fn ok(self, v: T) {
        self.send(Ok(v));
    }

    #[inline]
    pub fn err(self, e: EngineError) {
        self.send(Err(e));
    }

    #[inline]
    pub fn err_code(self, code: ErrorCode) {
        self.send(Err(EngineError::new(code)));
    }
}

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
    /// 'wx' – write+truncate, fail if path already exists
    WriteExclusive,
    /// 'wx+' – read+write+truncate, fail if path already exists
    ReadWriteExclusive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    /// Overwrite the file (create if missing).
    Overwrite,
    /// Append to the file (create if missing).
    Append,
}

#[derive(Debug)]
pub enum IOCmd {
    Access {
        path: String,
        resp: IOCmdResp<(bool, bool, u64)>,
    },

    Write {
        path: String,
        data: Vec<u8>,
        mode: WriteMode,
        resp: IOCmdResp<bool>,
    },

    WriteShared {
        path: String,
        store: v8::SharedRef<v8::BackingStore>,
        range: std::ops::Range<usize>,
        mode: WriteMode,
        resp: IOCmdResp<bool>,
    },

    Open {
        path: String,
        flag: OpenFlag,
        cleanup_path: Option<String>,
        synthetic_stat: Option<FileStat>,
        resp: IOCmdResp<FileId>,
    },

    Close {
        rid: FileId,
        resp: IOCmdResp<()>,
    },

    Copy {
        src_path: String,
        dest_path: String,
        resp: IOCmdResp<()>,
    },

    Fstat {
        rid: FileId,
        resp: IOCmdResp<FileStat>,
    },

    Ftruncate {
        rid: FileId,
        len: u64,
        resp: IOCmdResp<()>,
    },

    Mkdir {
        dir_path: String,
        recursive: bool,
        resp: IOCmdResp<()>,
    },

    Readdir {
        dir_path: String,
        resp: IOCmdResp<Vec<String>>,
    },

    Unlink {
        file_path: String,
        resp: IOCmdResp<()>,
    },

    Rename {
        old_path: String,
        new_path: String,
        resp: IOCmdResp<()>,
    },

    Rmdir {
        dir_path: String,
        recursive: bool,
        resp: IOCmdResp<()>,
    },

    Stat {
        path: String,
        recursive: bool,
        resp: IOCmdResp<StatResult>,
    },

    WriteFd {
        rid: FileId,
        data: Vec<u8>,
        position: Option<u64>,
        resp: IOCmdResp<usize>,
    },

    WriteFdShared {
        rid: FileId,
        store: v8::SharedRef<v8::BackingStore>,
        range: std::ops::Range<usize>,
        position: Option<u64>,
        resp: IOCmdResp<usize>,
    },

    ReadFd {
        rid: FileId,
        length: u64,
        position: Option<u64>,
        resp: IOCmdResp<Vec<u8>>,
    },

    ReadFile {
        path: String,
        position: Option<u64>,
        length: Option<u64>,
        resp: IOCmdResp<Vec<u8>>,
    },

    ReadCompressedFile {
        path: String,
        pack_data: Option<Vec<u8>>,
        resp: IOCmdResp<Vec<u8>>,
    },

    ReadZipEntry {
        zip_path: String,
        entries_json: String,
        pack_data: Option<Vec<u8>>,
        resp: IOCmdResp<Vec<ZipEntryResult>>,
    },

    /// Get file info: size + digest (md5/sha1/sha256).
    /// Returns (size_bytes, digest_hex_string).
    GetFileInfo {
        path: String,
        /// "md5", "sha1", or "sha256"
        algorithm: String,
        /// Pre-read bytes for pack-backed sources.  When `Some`, the handler
        /// computes the digest from these bytes instead of reading from `path`.
        pack_data: Option<Vec<u8>>,
        resp: IOCmdResp<(u64, String)>,
    },

    /// Read an image from `path` and convert it into a normalized RGBA8 buffer.
    /// Payload is (width, height, rgba_bytes).
    ///
    /// If `target_width` / `target_height` are set, the decoded image will be
    /// resized to fit within the target dimensions (preserving aspect ratio).
    /// This avoids decoding a 4096x4096 image at full resolution when only
    /// a 512x512 version is needed.
    ReadImageRgba8 {
        path: String,
        target_width: Option<u32>,
        target_height: Option<u32>,
        /// Mount generation for cache identity.
        cache_generation: u64,
        /// Pre-read bytes for pack-backed sources.
        pack_data: Option<Vec<u8>>,
        /// Per-game cache dir for derived texture cache. None = no derived caching.
        game_cache_dir: Option<String>,
        /// GPU compressed texture capabilities snapshot (session-level, not global).
        gpu_caps: crate::device::gpu_caps::GpuCapsSnapshot,
        /// Mount table for companion file lookup (pack-backed variant resolution).
        mount_table: Option<std::sync::Arc<crate::vfs::MountTable>>,
        resp: IOCmdResp<DecodedImage>,
    },

    /// Preload multiple images in parallel.
    /// Returns a list of results (path, Ok((width, height)) or Err(error_message)).
    PreloadImages {
        /// Each entry is `(path, mount_generation, optional_pack_data)`.  Per-path generation
        /// ensures /code paths use their mount generation while /user|/cache|/tmp
        /// paths use 0, without batch-level conflation.
        entries: Vec<(String, u64, Option<Vec<u8>>)>,
        /// Per-game cache dir for derived texture cache. None = no derived caching.
        game_cache_dir: Option<String>,
        /// GPU compressed texture capabilities snapshot (session-level, not global).
        gpu_caps: crate::device::gpu_caps::GpuCapsSnapshot,
        /// Mount table for companion file lookup (pack-backed variant resolution).
        mount_table: Option<std::sync::Arc<crate::vfs::MountTable>>,
        resp: IOCmdResp<Vec<(String, Result<(u32, u32), String>)>>,
    },

    /// Clear the image cache (useful for memory management)
    ClearImageCache {
        /// Per-game cache dir to clear derived texture cache.
        game_cache_dir: Option<String>,
        resp: IOCmdResp<()>,
    },

    /// Get image cache statistics
    GetImageCacheStats {
        resp: IOCmdResp<ImageCacheStats>,
    },

    /// Extract a zip file to destination directory.
    /// Returns the number of files extracted.
    Unzip {
        zip_path: String,
        dest_dir: String,
        resp: IOCmdResp<usize>,
    },

    /// Ingest a zip archive into a `.mpkg` package file.
    ///
    /// This replaces the old "extract to directory" flow: the zip is
    /// converted to the runtime-native package format in a single pass.
    IngestZipToPackage {
        zip_path: String,
        pkg_path: String,
        package_name: String,
        package_version: String,
        resp: IOCmdResp<()>,
    },

    // ── Storage (KV) ──────────────────────────────────────────────
    /// Read a storage file; returns content or empty string if not found.
    StorageGet {
        path: String,
        resp: IOCmdResp<String>,
    },

    /// Write a storage file after checking total-size limit.
    StorageSet {
        dir: String,
        path: String,
        data: String,
        max_total: usize,
        resp: IOCmdResp<()>,
    },

    /// Delete a storage file (silent on NotFound).
    StorageRemove {
        path: String,
        resp: IOCmdResp<()>,
    },

    /// Remove all files in the storage directory.
    StorageClear {
        dir: String,
        resp: IOCmdResp<()>,
    },

    /// Enumerate keys + sizes; returns a JSON string.
    StorageInfo {
        dir: String,
        limit_size_kb: u32,
        resp: IOCmdResp<String>,
    },

    /// List saved files (prefix-filtered readdir + stat in one round trip).
    ListSavedFiles {
        dir: String,
        prefix: String,
        /// Virtual dir prefix for constructing paths visible to JS (e.g. "/user").
        virtual_dir: String,
        resp: IOCmdResp<Vec<SavedFileInfo>>,
    },

    Shutdown,
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

/// Result for a single entry read from a zip archive.
#[derive(Debug)]
pub struct ZipEntryResult {
    pub path: String,
    /// Encoded string (text for entries with encoding, base64 for binary).
    pub data: Option<String>,
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
    /// Raw compressed block data for level 0.
    pub data: Arc<Vec<u8>>,
}

/// Decoded image: either RGBA pixels or GPU-compressed blocks.
#[derive(Debug, Clone)]
pub enum DecodedImage {
    Rgba(NormalizedImage),
    Compressed(CompressedImage),
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
    fn default() -> Self { Self::Normal }
}

impl DecodedImage {
    pub fn width(&self) -> u32 {
        match self {
            Self::Rgba(img) => img.width,
            Self::Compressed(img) => img.width,
        }
    }
    pub fn height(&self) -> u32 {
        match self {
            Self::Rgba(img) => img.height,
            Self::Compressed(img) => img.height,
        }
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
