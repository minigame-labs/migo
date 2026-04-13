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
