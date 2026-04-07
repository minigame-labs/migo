use std::{
    collections::HashMap,
    io::SeekFrom,
    ops::Range,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        OnceLock,
    },
};

use deno_core::v8::{BackingStore, SharedRef};
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::Semaphore,
    time::Instant,
};
use tracing::{debug, trace, warn};

/// Maximum number of concurrent image decode tasks across all callers
/// (ReadImageRgba8, PreloadImages, etc.).
const MAX_CONCURRENT_IMAGE_DECODES: usize = 3;

/// Global semaphore shared by all image-decode call sites so that
/// ReadImageRgba8 and PreloadImages compete for the same pool of permits.
fn image_decode_semaphore() -> &'static Semaphore {
    static SEM: OnceLock<Semaphore> = OnceLock::new();
    SEM.get_or_init(|| Semaphore::new(MAX_CONCURRENT_IMAGE_DECODES))
}

// ---------------------------------------------------------------------------
// IO byte-budget: limits peak native-heap usage of heavy tasks (decode,
// extract) without touching the global IO command channel.
// ---------------------------------------------------------------------------

/// Default budget: 48 MB.  With semaphore=3, three concurrent 4K RGBA decodes
/// (3840x2160x4 = 33 MB each) will exceed this and trigger backpressure.
/// This is intentional — the budget prevents native heap exhaustion on
/// low-RAM devices while the semaphore limits concurrency.
const DEFAULT_IO_BUDGET_BYTES: usize = 48 * 1024 * 1024;

/// Tracks aggregate bytes consumed by in-flight heavy IO tasks (image decode,
/// zip extract).  Light commands (file open/close/stat) do **not** go through
/// the budget — they use the existing unbounded IO channel directly.
struct IoBudget {
    active_bytes: AtomicUsize,
    max_bytes: usize,
    notify: tokio::sync::Notify,
}

impl IoBudget {
    fn new(max_bytes: usize) -> Self {
        Self {
            active_bytes: AtomicUsize::new(0),
            max_bytes,
            notify: tokio::sync::Notify::new(),
        }
    }

    /// Reserve `bytes` from the budget.
    ///
    /// - If `bytes` fits within remaining budget, succeeds immediately.
    /// - If budget is temporarily full, waits up to 200 ms per retry.
    /// - If `bytes > max_bytes` (oversized image), waits until **all**
    ///   other tasks finish (`active_bytes == 0`), then admits exactly
    ///   one oversized task.  This prevents infinite-wait deadlock.
    async fn acquire(&self, bytes: usize) -> IoBudgetGuard<'_> {
        loop {
            let current = self.active_bytes.load(Ordering::Acquire);
            // Normal case: fits within remaining budget.
            // Oversized case: bytes > max_bytes, admit only when nothing
            // else is active (exclusive access).
            let can_proceed = if bytes <= self.max_bytes {
                current.checked_add(bytes).map_or(false, |sum| sum <= self.max_bytes)
            } else {
                current == 0
            };
            if can_proceed {
                if self
                    .active_bytes
                    .compare_exchange_weak(
                        current,
                        current + bytes,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    return IoBudgetGuard {
                        budget: self,
                        bytes,
                    };
                }
                // CAS failed — retry immediately
                continue;
            }
            // Over budget — wait for a release notification (with timeout)
            tokio::time::timeout(
                std::time::Duration::from_millis(200),
                self.notify.notified(),
            )
            .await
            .ok();
        }
    }

    fn release(&self, bytes: usize) {
        self.active_bytes.fetch_sub(bytes, Ordering::Release);
        self.notify.notify_waiters();
    }

    /// Current usage for debug overlay / metrics.
    #[allow(dead_code)]
    fn active_bytes(&self) -> usize {
        self.active_bytes.load(Ordering::Relaxed)
    }
}

/// RAII guard that releases budget bytes on drop.
struct IoBudgetGuard<'a> {
    budget: &'a IoBudget,
    bytes: usize,
}

impl Drop for IoBudgetGuard<'_> {
    fn drop(&mut self) {
        self.budget.release(self.bytes);
    }
}

fn io_budget() -> &'static IoBudget {
    static BUDGET: OnceLock<IoBudget> = OnceLock::new();
    BUDGET.get_or_init(|| IoBudget::new(DEFAULT_IO_BUDGET_BYTES))
}

use shared::{
    error::{EngineError, ErrorCode, io_error_to_error_code},
    protocol::io_cmd::{
        DecodedImage, FileId, FileStat, IOCmd, IOCmdResp, NormalizedImage, OpenFlag,
        SavedFileInfo, StatEntry, StatResult, WriteMode, MAX_READ_LENGTH,
    },
};
#[cfg(feature = "zip-extract")]
use shared::protocol::io_cmd::ZipEntryResult;

#[cfg(feature = "zip-extract")]
use crate::zip_extract;
use crate::{fast_image_decoder, image_cache};

// ---------------------------------------------------------------------------
// Texture variant selection + decode
// ---------------------------------------------------------------------------
//
// Variant selection determines which representation of an image to use:
//
// 1. **Compressed** — KTX2 blocks uploaded directly to GPU (fastest).
//    Used when: source is KTX2, GPU supports the format, no resize requested.
//
// 2. **RGBA decode** — standard PNG/JPEG/WebP decode (universal).
//    Used when: source is a standard image, or as fallback for KTX2.
//
// 3. **Compressed upgrade** — standard image with a `.ktx2` companion on disk.
//    Used when: loading PNG/JPEG, a companion `.ktx2` exists AND GPU supports
//    the format AND no resize.  Transparent optimization; games don't need to
//    reference the KTX2 directly.
//
// Fallback convention (path-based):
//   `foo.ktx2` → try `foo.png`, `foo.jpg`, `foo.jpeg`, `foo.webp`
//   `foo.png`  → try `foo.ktx2` (compressed upgrade)
//
const VARIANT_PRIMARY_RGBA: u8 = 0;
const VARIANT_PRIMARY_COMPRESSED: u8 = 1;
const VARIANT_FALLBACK_RGBA: u8 = 2;
const VARIANT_COMPANION_COMPRESSED: u8 = 3;

/// Result of variant selection.  Tells the caller what to do with the image.
enum VariantDecision {
    /// Use compressed blocks from the primary KTX2 source.
    Compressed(shared::protocol::io_cmd::CompressedImage),
    /// Use compressed blocks from a companion KTX2 file.
    CompressedFromCompanion(shared::protocol::io_cmd::CompressedImage),
    /// Decode the given data as RGBA (optionally resize).
    DecodeRgba {
        data: Vec<u8>,
        path_hint: String,
        variant_kind: u8,
    },
}

impl VariantDecision {
    fn gpu_format(&self) -> u32 {
        match self {
            Self::Compressed(img) | Self::CompressedFromCompanion(img) => img.vk_format,
            Self::DecodeRgba { .. } => 0,
        }
    }

    fn variant_kind(&self) -> u8 {
        match self {
            Self::Compressed(_) => VARIANT_PRIMARY_COMPRESSED,
            Self::CompressedFromCompanion(_) => VARIANT_COMPANION_COMPRESSED,
            Self::DecodeRgba { variant_kind, .. } => *variant_kind,
        }
    }
}

/// Extract the VkFormat code from a KTX2 header format enum.
fn ktx2_vk_format_code(format: &crate::ktx2::VkFormat) -> u32 {
    match format {
        crate::ktx2::VkFormat::Etc2R8G8B8UnormBlock => 147,
        crate::ktx2::VkFormat::Etc2R8G8B8A8UnormBlock => 151,
        crate::ktx2::VkFormat::Astc4x4UnormBlock => 157,
        crate::ktx2::VkFormat::Astc6x6UnormBlock => 163,
        crate::ktx2::VkFormat::Astc8x8UnormBlock => 169,
        crate::ktx2::VkFormat::Unknown(_) => 0,
    }
}

/// Check whether the GPU supports a given VkFormat code.
fn gpu_supports_vk_format(vk_format: u32, gpu_caps: &shared::device::gpu_caps::GpuCapsSnapshot) -> bool {
    match vk_format {
        147 | 151 => gpu_caps.etc2,
        157 | 163 | 169 => gpu_caps.astc,
        _ => false,
    }
}

/// Try to parse KTX2 data and return a CompressedImage if the GPU supports it.
fn try_ktx2_as_compressed(
    data: &[u8],
    gpu_caps: &shared::device::gpu_caps::GpuCapsSnapshot,
) -> Option<shared::protocol::io_cmd::CompressedImage> {
    let ktx2 = crate::ktx2::parse_ktx2(data).ok()?;
    let vk_format = ktx2_vk_format_code(&ktx2.header.format);
    if vk_format == 0 || !gpu_supports_vk_format(vk_format, gpu_caps) {
        return None;
    }
    Some(shared::protocol::io_cmd::CompressedImage {
        width: ktx2.header.width,
        height: ktx2.header.height,
        vk_format,
        data: std::sync::Arc::new(ktx2.data.to_vec()),
    })
}

use shared::protocol::io_cmd::path_stem;

/// Convert an absolute path to a mount-table relative path.
/// Handles both `/code/` virtual paths and filesystem paths under code_dir.
fn mount_relative_path(path: &str, mt: &shared::vfs::MountTable) -> Option<String> {
    if let Some(relative) = path.strip_prefix("/code/") {
        return Some(relative.to_string());
    }
    let code_dir = mt.code_dir();
    std::path::Path::new(path)
        .strip_prefix(&code_dir)
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
}

/// Find a companion file by swapping the extension.
/// Returns `(path, file_data)` of the first match found.
///
/// Checks the mount table first (pack-backed), then the real filesystem.
fn find_companion(
    path: &str,
    try_extensions: &[&str],
    mount_table: Option<&shared::vfs::MountTable>,
) -> Option<(String, Vec<u8>)> {
    let stem = path_stem(path);
    for ext in try_extensions {
        let companion = format!("{}.{}", stem, ext);
        if let Some(mt) = mount_table {
            if let Some(relative) = mount_relative_path(&companion, mt) {
                if let Some(size) = mt.entry_size(&relative) {
                    if size <= MAX_READ_LENGTH {
                        if let Ok(data) = mt.read_range_limited(&relative, 0, None, MAX_READ_LENGTH) {
                            return Some((companion, data));
                        }
                    }
                }
            }
        }
        if let Ok(data) = read_filesystem_companion(&companion) {
            return Some((companion, data));
        }
    }
    None
}

/// Read a companion file from the filesystem with path-traversal protection.
///
/// Canonicalize → verify containment → open the validated canonical path.
/// Both canonicalize and open follow symlinks, but opening the canonical
/// path (not the original) minimises the TOCTOU window.
fn read_filesystem_companion(path: &str) -> std::io::Result<Vec<u8>> {
    let candidate = std::path::Path::new(path);
    let parent = candidate.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing parent")
    })?;
    let canonical_parent = std::fs::canonicalize(parent)?;
    let canonical_file = std::fs::canonicalize(candidate)?;
    if !canonical_file.starts_with(&canonical_parent) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "companion escapes parent directory",
        ));
    }
    let mut file = std::fs::File::open(&canonical_file)?;
    let size = file.metadata()?.len();
    if size > MAX_READ_LENGTH {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "companion exceeds read limit",
        ));
    }
    let mut buf = Vec::with_capacity(size as usize);
    std::io::Read::read_to_end(&mut file, &mut buf)?;
    Ok(buf)
}

fn max_variant_source_size(
    path: &str,
    primary_size: usize,
    mount_table: Option<&shared::vfs::MountTable>,
) -> usize {
    let mut max_size = primary_size;
    let stem = path_stem(path);
    for ext in VARIANT_EXTENSIONS {
        let candidate = format!("{}.{}", stem, ext);
        // Skip the primary path itself — caller already provided its size.
        if candidate == path { continue; }
        if let Ok(meta) = std::fs::metadata(&candidate) {
            max_size = max_size.max(meta.len() as usize);
            continue;
        }
        if let Some(mt) = mount_table {
            if let Some(relative) = mount_relative_path(&candidate, mt) {
                if let Some(size) = mt.entry_size(&relative) {
                    max_size = max_size.max(size as usize);
                }
            }
        }
    }
    max_size
}

/// Standard fallback extensions to try when KTX2 can't be used.
const RGBA_FALLBACK_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];
use shared::protocol::io_cmd::VARIANT_EXTENSIONS;

/// Select the best variant for an image load, then decode/prepare it.
///
/// This is the single entry point for all image decode paths (ReadImageRgba8
/// and PreloadImages).  It handles:
/// - KTX2 → compressed (if GPU supports + no resize)
/// - KTX2 → RGBA fallback (auto-finds companion PNG/JPEG)
/// - Standard image → compressed upgrade (auto-finds companion KTX2)
/// - Standard image → RGBA decode (+ optional resize)
fn select_variant(
    primary_path: &str,
    primary_data: Vec<u8>,
    gpu_caps: &shared::device::gpu_caps::GpuCapsSnapshot,
    has_resize: bool,
    mount_table: Option<&shared::vfs::MountTable>,
) -> Result<VariantDecision, EngineError> {
    // ── KTX2 primary ──────────────────────────────────────────────
    if crate::ktx2::is_ktx2(&primary_data) {
        // Can we use compressed blocks directly?
        if !has_resize {
            if let Some(compressed) = try_ktx2_as_compressed(&primary_data, gpu_caps) {
                tracing::info!(
                    "variant: compressed direct — {} ({}x{} fmt={})",
                    primary_path, compressed.width, compressed.height, compressed.vk_format,
                );
                return Ok(VariantDecision::Compressed(compressed));
            }
        }

        // Need fallback: KTX2 can't be used (GPU unsupported, resize, or parse error).
        // Try companion standard image (filesystem + mount table).
        if let Some((fb_path, fb_data)) = find_companion(primary_path, RGBA_FALLBACK_EXTENSIONS, mount_table) {
            tracing::info!("variant: RGBA fallback — {} -> {}", primary_path, fb_path);
            return Ok(VariantDecision::DecodeRgba {
                data: fb_data,
                path_hint: fb_path,
                variant_kind: VARIANT_FALLBACK_RGBA,
            });
        }

        // No fallback available — error.
        let reason = if has_resize {
            "resize requested but no standard-image fallback found"
        } else {
            "GPU does not support this compressed format and no fallback found"
        };
        return Err(EngineError::new(ErrorCode::ImageReadError).with_detail(
            format!("{} for '{}'", reason, primary_path)
        ));
    }

    // ── Standard image primary ────────────────────────────────────
    // Compressed upgrade: check for .ktx2 companion when GPU supports
    // compressed formats and no resize is needed.
    if !has_resize && (gpu_caps.etc2 || gpu_caps.astc) {
        if let Some((_, companion_data)) = find_companion(primary_path, &["ktx2"], mount_table) {
            if let Some(compressed) = try_ktx2_as_compressed(&companion_data, gpu_caps) {
                tracing::info!(
                    "variant: compressed upgrade — {} ({}x{} fmt={})",
                    primary_path, compressed.width, compressed.height, compressed.vk_format,
                );
                return Ok(VariantDecision::CompressedFromCompanion(compressed));
            }
        }
    }

    // Standard RGBA decode (possibly with resize).
    Ok(VariantDecision::DecodeRgba {
        data: primary_data,
        path_hint: primary_path.to_string(),
        variant_kind: VARIANT_PRIMARY_RGBA,
    })
}

/// Decode raw bytes as a standard RGBA image, optionally resizing.
fn decode_rgba(
    data: &[u8],
    path_hint: &str,
    has_resize: bool,
    target_width: Option<u32>,
    target_height: Option<u32>,
) -> Result<NormalizedImage, EngineError> {
    let mut img = fast_image_decoder::decode_image_fast(data, Some(path_hint))?;
    if has_resize {
        let tw = target_width.unwrap();
        let th = target_height.unwrap();
        if img.width > tw || img.height > th {
            img = fast_image_decoder::resize_image(img, tw, th);
        }
    }
    Ok(img)
}

/// Decode a variant decision into a `DecodedImage`.
fn decode_selected_variant(
    variant: VariantDecision,
    has_resize: bool,
    target_width: Option<u32>,
    target_height: Option<u32>,
) -> Result<DecodedImage, EngineError> {
    match variant {
        VariantDecision::Compressed(img)
        | VariantDecision::CompressedFromCompanion(img) => {
            Ok(DecodedImage::Compressed(img))
        }
        VariantDecision::DecodeRgba { data, path_hint, .. } => {
            let img = decode_rgba(&data, &path_hint, has_resize, target_width, target_height)?;
            Ok(DecodedImage::Rgba(img))
        }
    }
}

#[cfg(test)]
fn select_and_decode(
    primary_path: &str,
    primary_data: Vec<u8>,
    gpu_caps: &shared::device::gpu_caps::GpuCapsSnapshot,
    has_resize: bool,
    target_width: Option<u32>,
    target_height: Option<u32>,
    mount_table: Option<&shared::vfs::MountTable>,
) -> Result<(DecodedImage, u8, u32), EngineError> {
    let variant = select_variant(primary_path, primary_data, gpu_caps, has_resize, mount_table)?;
    let kind = variant.variant_kind();
    let fmt = variant.gpu_format();
    let image = decode_selected_variant(variant, has_resize, target_width, target_height)?;
    Ok((image, kind, fmt))
}

pub struct IoCmdHandler {
    next_id: FileId,
    free_ids: Vec<FileId>,
    files: HashMap<FileId, fs::File>,
    temp_files: HashMap<FileId, PathBuf>,
    synthetic_stats: HashMap<FileId, FileStat>,
    /// Cached total byte size per storage directory, avoiding O(n) re-scan
    /// on every `StorageSet`.  Populated lazily on first write, then
    /// maintained incrementally by Set / Remove / Clear operations.
    storage_totals: HashMap<PathBuf, usize>,
}

impl IoCmdHandler {
    /// Initial capacity for the file handle map.
    /// Most games use a small number of concurrent file handles.
    const INITIAL_FILE_CAPACITY: usize = 8;

    pub fn new() -> Self {
        Self {
            next_id: 3, // 0,1,2 reserved for stdio
            free_ids: Vec::new(),
            files: HashMap::with_capacity(Self::INITIAL_FILE_CAPACITY),
            temp_files: HashMap::new(),
            synthetic_stats: HashMap::new(),
            storage_totals: HashMap::new(),
        }
    }

    #[inline]
    fn alloc_id(&mut self) -> Result<FileId, EngineError> {
        if let Some(id) = self.free_ids.pop() {
            return Ok(id);
        }
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| EngineError::new(ErrorCode::ExceedMaxConcurrentFdLimit))?;
        Ok(id)
    }

    #[inline]
    fn send_resp<T>(resp: IOCmdResp<T>, result: Result<T, EngineError>) {
        resp.send(result)
    }

    #[inline]
    fn io_err(e: std::io::Error) -> EngineError {
        let detail = e.to_string();
        let code = io_error_to_error_code(&e);
        EngineError::new(code).with_detail(detail)
    }

    #[inline]
    fn code_err(code: ErrorCode) -> EngineError {
        EngineError::new(code)
    }

    pub fn close_all(&mut self) {
        self.files.clear();
        for (_, path) in self.temp_files.drain() {
            let _ = std::fs::remove_file(path);
        }
        self.synthetic_stats.clear();
        self.free_ids.clear();
        self.storage_totals.clear();
    }

    pub async fn handle_cmd(&mut self, cmd: IOCmd) {
        trace!("handle io cmd: {:?}", cmd);

        match cmd {
            IOCmd::Shutdown => unreachable!("Shutdown is handled by IOThread loop"),

            IOCmd::Access { path, resp } => {
                let r = fs::metadata(&path)
                    .await
                    .map(|m| (m.is_file(), m.is_dir(), m.len()))
                    .map_err(Self::io_err);
                Self::send_resp(resp, r);
            }

            IOCmd::Write {
                path,
                data,
                mode,
                resp,
            } => {
                let r = Self::write_file(&path, &data, mode).await;
                Self::send_resp(resp, r);
            }

            IOCmd::WriteShared {
                path,
                store,
                range,
                mode,
                resp,
            } => {
                let r = Self::write_shared(&path, &store, range, mode).await;
                Self::send_resp(resp, r);
            }

            IOCmd::Open {
                path,
                flag,
                cleanup_path,
                synthetic_stat,
                resp,
            } => {
                let r = self.open_file(&path, flag, cleanup_path, synthetic_stat).await;
                Self::send_resp(resp, r);
            }

            IOCmd::Close { rid, resp } => {
                let r = self.files.remove(&rid).map(|file| {
                    drop(file);
                    if let Some(path) = self.temp_files.remove(&rid) {
                        let _ = std::fs::remove_file(path);
                    }
                    self.synthetic_stats.remove(&rid);
                    self.free_ids.push(rid);
                })
                .ok_or_else(|| Self::code_err(ErrorCode::BadFileDescriptor));
                Self::send_resp(resp, r);
            }

            IOCmd::Copy {
                src_path,
                dest_path,
                resp,
            } => {
                let r = fs::copy(&src_path, &dest_path)
                    .await
                    .map(|_| ())
                    .map_err(Self::io_err);
                Self::send_resp(resp, r);
            }

            IOCmd::Fstat { rid, resp } => {
                let r = match self.synthetic_stats.get(&rid) {
                    Some(stat) => Ok(stat.clone()),
                    None => match self.files.get(&rid) {
                    Some(file) => match file.metadata().await {
                        Ok(meta) => Ok(Self::build_stat(meta)),
                        Err(e) => Err(Self::io_err(e)),
                    },
                    None => Err(Self::code_err(ErrorCode::BadFileDescriptor)),
                }};
                Self::send_resp(resp, r);
            }

            IOCmd::Ftruncate { rid, len, resp } => {
                let r: Result<(), EngineError> = (async {
                    let file = self
                        .files
                        .get_mut(&rid)
                        .ok_or_else(|| Self::code_err(ErrorCode::BadFileDescriptor))?;

                    file.set_len(len).await.map_err(Self::io_err)?;

                    // Best-effort move cursor to end.
                    let _ = file.seek(SeekFrom::End(0)).await;
                    Ok(())
                })
                .await;

                Self::send_resp(resp, r);
            }

            IOCmd::Mkdir {
                dir_path,
                recursive,
                resp,
            } => {
                let r = if recursive {
                    fs::create_dir_all(&dir_path).await
                } else {
                    fs::create_dir(&dir_path).await
                }
                .map_err(Self::io_err);
                Self::send_resp(resp, r);
            }

            // Readdir returns direct children (file and directory names), sorted.
            IOCmd::Readdir { dir_path, resp } => {
                let r: Result<Vec<String>, EngineError> = (async {
                    let mut entries = Vec::new();
                    let mut rd = fs::read_dir(&dir_path).await.map_err(Self::io_err)?;
                    while let Some(entry) = rd.next_entry().await.map_err(Self::io_err)? {
                        if let Some(name) = entry.file_name().to_str() {
                            entries.push(name.to_string());
                        }
                    }
                    entries.sort_unstable();
                    Ok(entries)
                })
                .await;
                Self::send_resp(resp, r);
            }

            IOCmd::Unlink { file_path, resp } => {
                let r = fs::remove_file(&file_path).await.map_err(Self::io_err);
                Self::send_resp(resp, r);
            }

            IOCmd::Rename {
                old_path,
                new_path,
                resp,
            } => {
                let r = fs::rename(&old_path, &new_path).await.map_err(Self::io_err);
                Self::send_resp(resp, r);
            }

            IOCmd::Rmdir {
                dir_path,
                recursive,
                resp,
            } => {
                let r = if recursive {
                    fs::remove_dir_all(&dir_path).await
                } else {
                    fs::remove_dir(&dir_path).await
                }
                .map_err(Self::io_err);
                Self::send_resp(resp, r);
            }

            IOCmd::Stat {
                path,
                recursive,
                resp,
            } => {
                let r = if !recursive {
                    match fs::metadata(&path).await {
                        Ok(meta) => Ok(StatResult::Single(Self::build_stat(meta))),
                        Err(e) => Err(Self::io_err(e)),
                    }
                } else {
                    Self::stat_dir_recursive(PathBuf::from(&path)).await
                };

                Self::send_resp(resp, r);
            }

            IOCmd::WriteFd {
                rid,
                data,
                position,
                resp,
            } => {
                let r: Result<usize, EngineError> = (async {
                    let file = self
                        .files
                        .get_mut(&rid)
                        .ok_or_else(|| Self::code_err(ErrorCode::BadFileDescriptor))?;

                    if let Some(pos) = position {
                        file.seek(SeekFrom::Start(pos))
                            .await
                            .map_err(Self::io_err)?;
                    }

                    file.write_all(&data).await.map_err(Self::io_err)?;
                    Ok(data.len())
                })
                .await;

                Self::send_resp(resp, r);
            }

            IOCmd::WriteFdShared {
                rid,
                store,
                range,
                position,
                resp,
            } => {
                let r: Result<usize, EngineError> = (async {
                    let file = self
                        .files
                        .get_mut(&rid)
                        .ok_or_else(|| Self::code_err(ErrorCode::BadFileDescriptor))?;

                    // Copy bytes first (never hold V8 memory across await).
                    let data = Self::copy_backing_store_range(&store, range)?;

                    if let Some(pos) = position {
                        file.seek(SeekFrom::Start(pos))
                            .await
                            .map_err(Self::io_err)?;
                    }

                    file.write_all(&data).await.map_err(Self::io_err)?;
                    Ok(data.len())
                })
                .await;

                Self::send_resp(resp, r);
            }

            IOCmd::ReadFd {
                rid,
                length,
                position,
                resp,
            } => {
                let r: Result<Vec<u8>, EngineError> = (async {
                    if length > MAX_READ_LENGTH {
                        return Err(EngineError::new(ErrorCode::InvalidArgument)
                            .with_detail(format!("read length {} exceeds limit {}", length, MAX_READ_LENGTH)));
                    }

                    let file = self
                        .files
                        .get_mut(&rid)
                        .ok_or_else(|| Self::code_err(ErrorCode::BadFileDescriptor))?;

                    if let Some(pos) = position {
                        file.seek(SeekFrom::Start(pos))
                            .await
                            .map_err(Self::io_err)?;
                    }

                    let mut buf = vec![0u8; length as usize];
                    let mut total = 0;
                    while total < buf.len() {
                        match file.read(&mut buf[total..]).await {
                            Ok(0) => break,
                            Ok(n) => total += n,
                            Err(e) => return Err(Self::io_err(e)),
                        }
                    }
                    buf.truncate(total);
                    Ok(buf)
                })
                .await;

                Self::send_resp(resp, r);
            }

            IOCmd::ReadFile {
                path,
                position,
                length,
                resp,
            } => {
                let r: Result<Vec<u8>, EngineError> = (async {
                    if let Some(len) = length {
                        if len > MAX_READ_LENGTH {
                            return Err(EngineError::new(ErrorCode::InvalidArgument)
                                .with_detail(format!("read length {} exceeds limit {}", len, MAX_READ_LENGTH)));
                        }
                    }

                    // Open file and seek/read specific range
                    let mut file = fs::File::open(&path).await.map_err(Self::io_err)?;

                    // When length is not specified, check that the remaining
                    // bytes from position to EOF don't exceed the limit.
                    // This closes the gap where position=Some + length=None
                    // could read arbitrarily large tails.
                    if length.is_none() {
                        let meta = file.metadata().await.map_err(Self::io_err)?;
                        let file_len = meta.len();
                        let remaining = file_len.saturating_sub(position.unwrap_or(0));
                        if remaining > MAX_READ_LENGTH {
                            return Err(EngineError::new(ErrorCode::InvalidArgument)
                                .with_detail(format!(
                                    "remaining file size {} exceeds limit {}",
                                    remaining, MAX_READ_LENGTH
                                )));
                        }
                    }

                    // Seek to position if specified
                    if let Some(pos) = position {
                        file.seek(SeekFrom::Start(pos))
                            .await
                            .map_err(Self::io_err)?;
                    }

                    // Read specified length or rest of file
                    let data = if let Some(len) = length {
                        let mut buf = vec![0u8; len as usize];
                        let mut total = 0;
                        while total < buf.len() {
                            match file.read(&mut buf[total..]).await {
                                Ok(0) => break, // EOF
                                Ok(n) => total += n,
                                Err(e) => return Err(Self::io_err(e)),
                            }
                        }
                        buf.truncate(total);
                        buf
                    } else {
                        Self::read_file_to_end_limited(&mut file, MAX_READ_LENGTH).await?
                    };

                    Ok(data)
                })
                .await;

                Self::send_resp(resp, r);
            }

            // --- Heavy ops: spawned concurrently so the IO loop is not blocked ---

            #[cfg(feature = "compress-brotli")]
            IOCmd::ReadCompressedFile { path, pack_data, resp } => {
                tokio::spawn(async move {
                    let r: Result<Vec<u8>, EngineError> = async {
                        tokio::task::spawn_blocking(move || {
                            let source: Box<dyn std::io::Read> = match pack_data {
                                Some(data) => Box::new(std::io::Cursor::new(data)),
                                None => Box::new(std::io::BufReader::new(std::fs::File::open(&path).map_err(Self::io_err)?)),
                            };
                            let mut reader = brotli::Decompressor::new(source, 4096);
                            Self::read_to_end_limited(&mut reader, MAX_READ_LENGTH, "brotli output size")
                        })
                        .await
                        .map_err(|e| {
                            EngineError::new(ErrorCode::IoError)
                                .with_detail(format!("task join error: {}", e))
                        })?
                    }
                    .await;
                    resp.send(r);
                });
            }

            #[cfg(not(feature = "compress-brotli"))]
            IOCmd::ReadCompressedFile { resp, .. } => {
                resp.send(Err(EngineError::new(ErrorCode::IoError)
                    .with_detail("brotli decompression not available (compress-brotli feature disabled)")));
            }

            #[cfg(feature = "zip-extract")]
            IOCmd::ReadZipEntry {
                zip_path,
                entries_json,
                pack_data,
                resp,
            } => {
                tokio::spawn(async move {
                    let r = match tokio::task::spawn_blocking(move || {
                        match pack_data {
                            Some(data) => IoCmdHandler::read_zip_entries_from_reader(std::io::Cursor::new(data), &entries_json),
                            None => IoCmdHandler::read_zip_entries(&zip_path, &entries_json),
                        }
                    })
                    .await
                    {
                        Ok(inner) => inner,
                        Err(e) => Err(EngineError::new(ErrorCode::IoError)
                            .with_detail(format!("task join error: {}", e))),
                    };
                    resp.send(r);
                });
            }

            #[cfg(not(feature = "zip-extract"))]
            IOCmd::ReadZipEntry { resp, .. } => {
                Self::send_resp(
                    resp,
                    Err(EngineError::new(ErrorCode::IoError)
                        .with_msg("readZipEntry not available (zip feature disabled)")),
                );
            }

            IOCmd::GetFileInfo {
                path,
                algorithm,
                pack_data,
                resp,
            } => {
                tokio::spawn(async move {
                    let r = tokio::task::spawn_blocking(move || {
                        match pack_data {
                            Some(data) => IoCmdHandler::get_file_info_from_bytes(&data, &algorithm),
                            None => IoCmdHandler::get_file_info(&path, &algorithm),
                        }
                    })
                    .await
                    .map_err(|e| {
                        EngineError::new(ErrorCode::IoError)
                            .with_detail(format!("task join error: {e}"))
                    })
                    .and_then(|inner| inner);
                    resp.send(r);
                });
            }

            IOCmd::ReadImageRgba8 { path, target_width, target_height, cache_generation, pack_data, game_cache_dir, gpu_caps, mount_table, resp } => {
                // Resize requires BOTH dimensions.  Treat partial as no-resize
                // to avoid asymmetric cache/decode behavior.
                let has_resize = target_width.is_some() && target_height.is_some();
                // Structured cache key: (path, generation) — no delimiter collision.
                let io_cache_key: image_cache::ImageCacheKey = (path.clone(), cache_generation);
                // LRU cache fast path (full-resolution decodes only).
                if !has_resize {
                    if let Some(cached) = image_cache::global_cache().get(&io_cache_key) {
                        debug!("ReadImageRgba8 cache hit: {} g{}", path, cache_generation);
                        Self::send_resp(resp, Ok(DecodedImage::Rgba(cached.image)));
                        return;
                    }
                }

                tokio::spawn(async move {
                    let start = Instant::now();

                    // Budget estimation: use pack_data length if available (pack-backed),
                    // else file metadata.  Avoids fixed-constant fallback for pack paths.
                    let primary_size = if let Some(ref pd) = pack_data {
                        pd.len()
                    } else {
                        std::fs::metadata(&path).map(|m| m.len() as usize).unwrap_or(2048 * 2048 * 4)
                    };
                    let pre_estimate = max_variant_source_size(&path, primary_size, mount_table.as_deref())
                        .saturating_mul(16)
                        .clamp(16 * 1024, 256 * 1024 * 1024);
                    let _budget = io_budget().acquire(pre_estimate).await;

                    // Limit concurrent decodes to prevent Java Heap OOM on Android.
                    // safe: semaphore is never closed
                    let _permit = image_decode_semaphore().acquire().await.unwrap();

                    let path_clone = path.clone();
                    let gcd = game_cache_dir.clone();
                    let mt = mount_table.clone();
                    let task = tokio::task::spawn_blocking(
                        move || -> Result<(DecodedImage, u8), EngineError> {
                            let data = match pack_data {
                                Some(d) => d,
                                None => std::fs::read(&path_clone).map_err(|e| {
                                    EngineError::new(ErrorCode::ImageReadError)
                                        .with_detail(format!("failed to read file: {}", e))
                                })?,
                            };

                            let tw = target_width.unwrap_or(0);
                            let th = target_height.unwrap_or(0);
                            let variant = select_variant(
                                &path_clone, data, &gpu_caps,
                                has_resize,
                                mt.as_deref(),
                            )?;

                            let variant_kind = variant.variant_kind();
                            let cache_fmt = variant.gpu_format();

                            let cache_key = crate::derived_cache::DerivedKey {
                                asset_path: path_clone.clone(),
                                source_generation: cache_generation,
                                gpu_format: cache_fmt,
                                variant_kind,
                                target_width: tw,
                                target_height: th,
                            };

                            if let Some(ref cache_dir) = gcd {
                                if let Some(cached) = crate::derived_cache::load_derived(
                                    std::path::Path::new(cache_dir), &cache_key,
                                ) {
                                    tracing::debug!("derived cache hit: {}", path_clone);
                                    return Ok((cached, variant_kind));
                                }
                            }

                            let result = decode_selected_variant(
                                variant,
                                has_resize,
                                target_width,
                                target_height,
                            )?;

                            if let Some(ref cache_dir) = gcd {
                                crate::derived_cache::save_derived(
                                    std::path::Path::new(cache_dir), &cache_key, &result,
                                );
                            }

                            Ok((result, variant_kind))
                        },
                    );

                    let r = match task.await {
                        Ok(Ok((decoded, _variant_kind))) => {
                            if !has_resize {
                                if let DecodedImage::Rgba(ref rgba_img) = decoded {
                                    image_cache::global_cache().insert(
                                        (path.clone(), cache_generation),
                                        rgba_img.clone(),
                                    );
                                }
                            }
                            debug!(
                                "ReadImageRgba8: {} ({}x{}) {:?} in {:.2?}",
                                path, decoded.width(), decoded.height(),
                                match &decoded { DecodedImage::Rgba(_) => "RGBA", DecodedImage::Compressed(_) => "compressed" },
                                start.elapsed()
                            );
                            Ok(decoded)
                        }
                        Ok(Err(e)) => {
                            warn!("ReadImageRgba8 decode error: {:?}", e);
                            Err(e)
                        }
                        Err(join_err) => {
                            warn!("ReadImageRgba8 spawn_blocking join error: {join_err}");
                            Err(EngineError::new(ErrorCode::ImageReadError)
                                .with_detail(format!("spawn_blocking join error: {join_err}")))
                        }
                    };
                    resp.send(r);
                });
            }

            IOCmd::PreloadImages { entries, game_cache_dir, gpu_caps, mount_table, resp } => {
                tokio::spawn(async move {
                    let start = Instant::now();
                    let total = entries.len();
                    debug!("PreloadImages: {} images", total);

                    // Pre-allocate output slots to preserve input order.
                    // Cache hits fill their slot immediately; decode misses
                    // are dispatched and filled after completion.
                    type PreloadResult = (String, Result<(u32, u32), String>);
                    let mut slots: Vec<Option<PreloadResult>> = vec![None; total];

                    // (input_index, path, generation, optional_pack_data) for entries that need decoding.
                    let mut decode_tasks: Vec<(usize, String, u64, Option<Vec<u8>>)> = Vec::new();
                    {
                        let mut cache = image_cache::global_cache();
                        for (i, (path, generation, pack_data)) in entries.into_iter().enumerate() {
                            let key: image_cache::ImageCacheKey = (path.clone(), generation);
                            if let Some(cached) = cache.get(&key) {
                                slots[i] = Some((
                                    path,
                                    Ok((cached.image.width, cached.image.height)),
                                ));
                            } else {
                                decode_tasks.push((i, path, generation, pack_data));
                            }
                        }
                    }

                    // Spawn decode tasks, each remembering its input index.
                    let handles: Vec<(usize, String, _)> = decode_tasks
                        .into_iter()
                        .map(|(idx, path, cg, pack_data)| {
                            let path_for_error = path.clone();
                            let gcd = game_cache_dir.clone();
                            let mt = mount_table.clone();
                            let handle = tokio::spawn(async move {
                                let primary_size = pack_data
                                    .as_ref()
                                    .map(|d| d.len())
                                    .or_else(|| std::fs::metadata(&path).map(|m| m.len() as usize).ok())
                                    .unwrap_or(2048 * 2048 * 4);
                                let pre_estimate = max_variant_source_size(&path, primary_size, mt.as_deref())
                                    .saturating_mul(16)
                                    .clamp(16 * 1024, 256 * 1024 * 1024);
                                let _budget = io_budget().acquire(pre_estimate).await;
                                let _permit = image_decode_semaphore().acquire().await.unwrap();
                                tokio::task::spawn_blocking(move || {
                                    let data = match pack_data {
                                        Some(d) => d,
                                        None => match std::fs::read(&path) {
                                            Ok(d) => d,
                                            Err(e) => return (path, Err(format!("read error: {}", e))),
                                        },
                                    };

                                    match select_variant(&path, data, &gpu_caps, false, mt.as_deref()) {
                                        Ok(variant) => {
                                            let variant_kind = variant.variant_kind();
                                            let cache_fmt = variant.gpu_format();
                                            let cache_key = crate::derived_cache::DerivedKey {
                                                asset_path: path.clone(),
                                                source_generation: cg,
                                                gpu_format: cache_fmt,
                                                variant_kind,
                                                target_width: 0,
                                                target_height: 0,
                                            };

                                            if let Some(ref cache_dir) = gcd {
                                                if let Some(cached) = crate::derived_cache::load_derived(
                                                    std::path::Path::new(cache_dir), &cache_key,
                                                ) {
                                                    tracing::debug!("PreloadImages derived cache hit: {}", path);
                                                    let dims = (cached.width(), cached.height());
                                                    if let DecodedImage::Rgba(ref rgba) = cached {
                                                        image_cache::global_cache()
                                                            .insert((path.clone(), cg), rgba.clone());
                                                    }
                                                    return (path, Ok(dims));
                                                }
                                            }

                                            let decoded = match decode_selected_variant(
                                                variant, false, None, None,
                                            ) {
                                                Ok(v) => v,
                                                Err(e) => return (path, Err(format!("{:?}", e))),
                                            };

                                            let dims = (decoded.width(), decoded.height());
                                            if let DecodedImage::Rgba(ref rgba) = decoded {
                                                image_cache::global_cache()
                                                    .insert((path.clone(), cg), rgba.clone());
                                            }
                                            if let Some(ref cache_dir) = gcd {
                                                crate::derived_cache::save_derived(
                                                    std::path::Path::new(cache_dir), &cache_key, &decoded,
                                                );
                                            }
                                            (path, Ok(dims))
                                        }
                                        Err(e) => (path, Err(format!("{:?}", e))),
                                    }
                                })
                                .await
                            });
                            (idx, path_for_error, handle)
                        })
                        .collect();

                    // Await all decode tasks and fill their slots.
                    for (idx, fallback_path, handle) in handles {
                        let result: PreloadResult = match handle.await {
                            Ok(Ok(r)) => r,
                            Ok(Err(join_err)) => {
                                warn!("PreloadImages spawn_blocking join error: {}", join_err);
                                (fallback_path, Err(format!("decode task error: {}", join_err)))
                            }
                            Err(task_err) => {
                                warn!("PreloadImages task panic/cancel: {}", task_err);
                                (fallback_path, Err(format!("task error: {}", task_err)))
                            }
                        };
                        slots[idx] = Some(result);
                    }

                    // Unwrap — every slot is now filled (cache hit or decode).
                    let results: Vec<PreloadResult> = slots
                        .into_iter()
                        .map(|s| s.expect("[BUG] PreloadImages: unfilled slot"))
                        .collect();

                    debug!(
                        "PreloadImages completed: {}/{} images in {:.2?}",
                        results.len(),
                        total,
                        start.elapsed()
                    );
                    resp.send(Ok(results));
                });
            }

            IOCmd::ClearImageCache { game_cache_dir, resp } => {
                image_cache::global_cache().clear();
                // Also clear per-game derived texture cache.
                if let Some(dir) = game_cache_dir {
                    let derived = crate::derived_cache::derived_cache_dir(std::path::Path::new(&dir));
                    if derived.exists() {
                        let _ = std::fs::remove_dir_all(&derived);
                        debug!("Derived texture cache cleared: {}", derived.display());
                    }
                }
                debug!("Image cache cleared");
                Self::send_resp(resp, Ok(()));
            }

            IOCmd::GetImageCacheStats { resp } => {
                use shared::protocol::io_cmd::ImageCacheStats;
                let stats = image_cache::global_cache().stats();
                let result = ImageCacheStats {
                    entries: stats.entries,
                    size_bytes: stats.size_bytes,
                    max_bytes: stats.max_bytes,
                    hits: stats.hits,
                    misses: stats.misses,
                    hit_rate: stats.hit_rate(),
                };
                Self::send_resp(resp, Ok(result));
            }

            #[cfg(feature = "zip-extract")]
            IOCmd::Unzip {
                zip_path,
                dest_dir,
                resp,
            } => {
                tokio::spawn(async move {
                    let start = Instant::now();
                    debug!("Unzip: {} -> {}", zip_path, dest_dir);

                    let zip_path_clone = zip_path.clone();
                    let dest_dir_clone = dest_dir.clone();

                    let task = tokio::task::spawn_blocking(move || {
                        let zip_path = PathBuf::from(&zip_path_clone);
                        let dest_dir = PathBuf::from(&dest_dir_clone);

                        let file_count =
                            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
                        let file_count_clone = file_count.clone();

                        let progress_cb =
                            Box::new(move |_prog: f32, current: usize, _total: usize| {
                                file_count_clone
                                    .store(current, std::sync::atomic::Ordering::Relaxed);
                            });

                        match zip_extract::extract_zip(&zip_path, &dest_dir, Some(progress_cb)) {
                            Ok(()) => {
                                Ok(file_count.load(std::sync::atomic::Ordering::Relaxed))
                            }
                            Err(e) => Err(
                                EngineError::new(ErrorCode::IoError).with_detail(e.to_string())
                            ),
                        }
                    });

                    let r = match task.await {
                        Ok(result) => {
                            debug!("Unzip completed in {:.2?}", start.elapsed());
                            result
                        }
                        Err(join_err) => {
                            warn!("Unzip spawn_blocking join error: {join_err}");
                            Err(EngineError::new(ErrorCode::IoError)
                                .with_detail(format!("spawn_blocking join error: {join_err}")))
                        }
                    };
                    resp.send(r);
                });
            }

            #[cfg(not(feature = "zip-extract"))]
            IOCmd::Unzip { resp, .. } => {
                Self::send_resp(
                    resp,
                    Err(EngineError::new(ErrorCode::Unsupported)
                        .with_detail("zip-extract feature is not enabled")),
                );
            }

            #[cfg(feature = "zip-extract")]
            IOCmd::IngestZipToPackage {
                zip_path, pkg_path, package_name, package_version, resp,
            } => {
                tokio::spawn(async move {
                    let zp = zip_path.clone();
                    let pp = pkg_path.clone();
                    let pn = package_name;
                    let pv = package_version;
                    let task = tokio::task::spawn_blocking(move || {
                        crate::package_ingest::ingest_zip_to_package(
                            std::path::Path::new(&zp),
                            std::path::Path::new(&pp),
                            &pn,
                            &pv,
                        )
                    });
                    let r = match task.await {
                        Ok(Ok(_identity)) => {
                            debug!("IngestZipToPackage: {} -> {}", zip_path, pkg_path);
                            Ok(())
                        }
                        Ok(Err(e)) => {
                            warn!("IngestZipToPackage failed: {}", e);
                            Err(EngineError::new(ErrorCode::IoError).with_detail(e.to_string()))
                        }
                        Err(join_err) => {
                            warn!("IngestZipToPackage join error: {}", join_err);
                            Err(EngineError::new(ErrorCode::IoError)
                                .with_detail(format!("join error: {join_err}")))
                        }
                    };
                    resp.send(r);
                });
            }

            #[cfg(not(feature = "zip-extract"))]
            IOCmd::IngestZipToPackage { resp, .. } => {
                Self::send_resp(
                    resp,
                    Err(EngineError::new(ErrorCode::Unsupported)
                        .with_detail("zip-extract feature is not enabled")),
                );
            }

            // ── Storage (KV) ─────────────────────────────────────────
            IOCmd::StorageGet { path, resp } => {
                let r = match fs::read_to_string(&path).await {
                    Ok(content) => Ok(content),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
                    Err(e) => Err(Self::io_err(e)),
                };
                Self::send_resp(resp, r);
            }

            IOCmd::StorageSet {
                dir,
                path,
                data,
                max_total,
                resp,
            } => {
                let r: Result<(), EngineError> = (async {
                    fs::create_dir_all(&dir).await.map_err(Self::io_err)?;

                    // Existing size of the target key (0 if new).
                    let existing_size = fs::metadata(&path)
                        .await
                        .map(|m| m.len() as usize)
                        .unwrap_or(0);

                    // Use cached total if available, otherwise do a full scan
                    // and cache the result for subsequent writes.
                    let dir_key = PathBuf::from(&dir);
                    let total = match self.storage_totals.get(&dir_key) {
                        Some(&cached) => cached,
                        None => {
                            let mut sum: usize = 0;
                            let mut rd = fs::read_dir(&dir).await.map_err(Self::io_err)?;
                            while let Some(entry) = rd.next_entry().await.map_err(Self::io_err)? {
                                sum += entry
                                    .metadata()
                                    .await
                                    .map(|m| m.len() as usize)
                                    .unwrap_or(0);
                            }
                            self.storage_totals.insert(dir_key.clone(), sum);
                            sum
                        }
                    };

                    if total.saturating_sub(existing_size) + data.len() > max_total {
                        return Err(EngineError::new(ErrorCode::IoError)
                            .with_detail("setStorage:fail storage limit exceeded"));
                    }

                    fs::write(&path, &data).await.map_err(Self::io_err)?;

                    let new_total = total.saturating_sub(existing_size) + data.len();
                    self.storage_totals.insert(dir_key, new_total);
                    Ok(())
                })
                .await;
                Self::send_resp(resp, r);
            }

            IOCmd::StorageRemove { path, resp } => {
                // Only query file size when the cache is populated for this
                // directory — avoids an extra syscall when it would be wasted.
                let parent_key = PathBuf::from(&path);
                let parent_key = parent_key.parent().map(|p| p.to_path_buf());
                let need_size = parent_key
                    .as_ref()
                    .is_some_and(|k| self.storage_totals.contains_key(k));
                let removed_size = if need_size {
                    fs::metadata(&path)
                        .await
                        .map(|m| m.len() as usize)
                        .unwrap_or(0)
                } else {
                    0
                };
                let r = match fs::remove_file(&path).await {
                    Ok(()) => {
                        if let (Some(key), true) = (parent_key, removed_size > 0) {
                            if let Some(total) = self.storage_totals.get_mut(&key) {
                                *total = total.saturating_sub(removed_size);
                            }
                        }
                        Ok(())
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(Self::io_err(e)),
                };
                Self::send_resp(resp, r);
            }

            IOCmd::StorageClear { dir, resp } => {
                let r: Result<(), EngineError> = (async {
                    match fs::read_dir(&dir).await {
                        Ok(mut rd) => {
                            while let Some(entry) = rd.next_entry().await.map_err(Self::io_err)? {
                                let _ = fs::remove_file(entry.path()).await;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(Self::io_err(e)),
                    }
                    self.storage_totals.insert(PathBuf::from(&dir), 0);
                    Ok(())
                })
                .await;
                Self::send_resp(resp, r);
            }

            IOCmd::StorageInfo {
                dir,
                limit_size_kb,
                resp,
            } => {
                let r: Result<String, EngineError> = (async {
                    let mut keys: Vec<String> = Vec::new();
                    let mut total_bytes: u64 = 0;

                    match fs::read_dir(&dir).await {
                        Ok(mut rd) => {
                            while let Some(entry) = rd.next_entry().await.map_err(Self::io_err)? {
                                if let Some(name) = entry.file_name().to_str() {
                                    if let Some(key) = Self::hex_to_key(name) {
                                        keys.push(key);
                                    }
                                }
                                total_bytes +=
                                    entry.metadata().await.map(|m| m.len()).unwrap_or(0);
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(Self::io_err(e)),
                    }

                    let keys_json: String = keys
                        .iter()
                        .map(|k| format!("\"{}\"", Self::json_escape(k)))
                        .collect::<Vec<_>>()
                        .join(",");

                    let current_size_kb = (total_bytes + 1023) / 1024;

                    Ok(format!(
                        "{{\"keys\":[{keys_json}],\"currentSize\":{current_size_kb},\"limitSize\":{limit_size_kb}}}"
                    ))
                })
                .await;
                Self::send_resp(resp, r);
            }

            IOCmd::ListSavedFiles {
                dir,
                prefix,
                virtual_dir,
                resp,
            } => {
                let r: Result<Vec<SavedFileInfo>, EngineError> = (async {
                    let mut file_list = Vec::new();
                    let mut rd = match fs::read_dir(&dir).await {
                        Ok(rd) => rd,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            return Ok(file_list);
                        }
                        Err(e) => return Err(Self::io_err(e)),
                    };
                    while let Some(entry) = rd.next_entry().await.map_err(Self::io_err)? {
                        let name = match entry.file_name().to_str() {
                            Some(n) => n.to_string(),
                            None => continue,
                        };
                        if !name.starts_with(&prefix) {
                            continue;
                        }
                        if let Ok(meta) = entry.metadata().await {
                            if !meta.is_file() {
                                continue;
                            }
                            let mtime = meta
                                .modified()
                                .ok()
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            file_list.push(SavedFileInfo {
                                file_path: format!("{}/{}", virtual_dir, name),
                                size: meta.len(),
                                create_time: mtime,
                            });
                        }
                    }
                    Ok(file_list)
                })
                .await;
                Self::send_resp(resp, r);
            }
        }
    }

    async fn open_file(
        &mut self,
        path: &str,
        flag: OpenFlag,
        cleanup_path: Option<String>,
        synthetic_stat: Option<FileStat>,
    ) -> Result<FileId, EngineError> {
        let mut opts = OpenOptions::new();

        match flag {
            OpenFlag::Read => {
                opts.read(true);
            }
            OpenFlag::ReadWrite => {
                opts.read(true).write(true);
            }
            OpenFlag::WriteTruncateCreate => {
                opts.write(true).create(true).truncate(true);
            }
            OpenFlag::ReadWriteTruncateCreate => {
                opts.read(true).write(true).create(true).truncate(true);
            }
            OpenFlag::AppendCreate => {
                opts.append(true).create(true);
            }
            OpenFlag::ReadAppendCreate => {
                opts.read(true).append(true).create(true);
            }
            OpenFlag::AppendExclusive => {
                opts.append(true).create_new(true);
            }
            OpenFlag::ReadAppendExclusive => {
                opts.read(true).append(true).create_new(true);
            }
            OpenFlag::AppendSyncCreate => {
                // 'as' – sync hint; treated as append+create (sync is implicit in our model)
                opts.append(true).create(true);
            }
            OpenFlag::ReadAppendSyncCreate => {
                // 'as+' – sync hint; treated as read+append+create
                opts.read(true).append(true).create(true);
            }
            OpenFlag::WriteExclusive => {
                opts.write(true).create_new(true);
            }
            OpenFlag::ReadWriteExclusive => {
                opts.read(true).write(true).create_new(true);
            }
        }

        let file = opts.open(path).await.map_err(Self::io_err)?;
        let id = self.alloc_id()?;
        self.files.insert(id, file);
        if let Some(path) = cleanup_path {
            self.temp_files.insert(id, PathBuf::from(path));
        }
        if let Some(stat) = synthetic_stat {
            self.synthetic_stats.insert(id, stat);
        }
        Ok(id)
    }

    async fn write_file(path: &str, data: &[u8], mode: WriteMode) -> Result<bool, EngineError> {
        match mode {
            WriteMode::Overwrite => fs::write(path, data)
                .await
                .map(|_| true)
                .map_err(Self::io_err),

            WriteMode::Append => {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .await
                    .map_err(Self::io_err)?;

                file.write_all(data)
                    .await
                    .map(|_| true)
                    .map_err(Self::io_err)
            }
        }
    }

    async fn write_shared(
        path: &str,
        store: &SharedRef<BackingStore>,
        range: Range<usize>,
        mode: WriteMode,
    ) -> Result<bool, EngineError> {
        let data = Self::copy_backing_store_range(store, range)?;
        Self::write_file(path, &data, mode).await
    }

    /// Copy a byte range out of a V8 BackingStore safely.
    /// Uses `byte_length()` (fix for NonNull<c_void> no `.len()`).
    fn copy_backing_store_range(
        store: &SharedRef<BackingStore>,
        range: Range<usize>,
    ) -> Result<Vec<u8>, EngineError> {
        let nn = store
            .data()
            .ok_or_else(|| EngineError::new(ErrorCode::ArrayBufferDoesNotExist))?;
        let total = store.byte_length();

        if range.start > range.end || range.end > total {
            return Err(EngineError::new(ErrorCode::InvalidArgument)
                .with_detail(format!("invalid range: {:?}, total={}", range, total)));
        }

        let len = range.end - range.start;
        let ptr = nn.as_ptr() as *const u8;

        // SAFETY: bounds validated by byte_length.
        let slice = unsafe { std::slice::from_raw_parts(ptr.add(range.start), len) };
        Ok(slice.to_vec())
    }

    #[inline]
    fn build_stat(meta: std::fs::Metadata) -> FileStat {
        let mode = Self::get_mode(&meta);
        let size = meta.len();

        let atime = meta
            .accessed()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        FileStat {
            mode,
            size,
            atime,
            mtime,
            is_file: meta.is_file(),
            is_directory: meta.is_dir(),
        }
    }

    #[inline]
    fn get_mode(meta: &std::fs::Metadata) -> u32 {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            meta.mode()
        }
        #[cfg(not(unix))]
        {
            if meta.permissions().readonly() {
                0o444
            } else {
                0o666
            }
        }
    }

    // ── Storage helpers ────────────────────────────────────────

    /// Decode a hex filename back to the original storage key.
    fn hex_to_key(hex: &str) -> Option<String> {
        let hex = hex.as_bytes();
        if hex.len() % 2 != 0 {
            return None;
        }
        let mut bytes = Vec::with_capacity(hex.len() / 2);
        for pair in hex.chunks_exact(2) {
            let hi = Self::hex_digit(pair[0])?;
            let lo = Self::hex_digit(pair[1])?;
            bytes.push((hi << 4) | lo);
        }
        String::from_utf8(bytes).ok()
    }

    #[inline]
    fn hex_digit(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }

    /// Escape a string for safe embedding in a JSON string literal.
    fn json_escape(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => out.push(c),
            }
        }
        out
    }

    /// Stat recursively for files under a directory.
    async fn stat_dir_recursive(
        root: PathBuf,
    ) -> Result<StatResult, EngineError> {
        use std::collections::BTreeMap;

        let root_meta = fs::metadata(&root).await.map_err(Self::io_err)?;
        if root_meta.is_file() {
            return Ok(StatResult::Single(Self::build_stat(root_meta)));
        }

        // Use BTreeMap for automatic sorting by key, avoiding O(n log n) sort at the end
        let mut out: BTreeMap<String, FileStat> = BTreeMap::new();
        let mut stack: Vec<PathBuf> = vec![root.clone()];

        while let Some(dir) = stack.pop() {
            let mut rd = fs::read_dir(&dir).await.map_err(Self::io_err)?;

            while let Some(entry) = rd.next_entry().await.map_err(Self::io_err)? {
                let path = entry.path();
                let ft = entry.file_type().await.map_err(Self::io_err)?;

                if ft.is_dir() {
                    stack.push(path);
                } else if ft.is_file() {
                    let meta = entry.metadata().await.map_err(Self::io_err)?;
                    let stat = Self::build_stat(meta);
                    let rel = path
                        .strip_prefix(&root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned();

                    out.insert(rel, stat);
                }
            }
        }

        // BTreeMap iteration is already sorted by key
        Ok(StatResult::Recursive(
            out.into_iter()
                .map(|(path, stat)| StatEntry { path, stat })
                .collect(),
        ))
    }

    async fn read_file_to_end_limited(
        file: &mut fs::File,
        max_len: u64,
    ) -> Result<Vec<u8>, EngineError> {
        let mut out = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = file.read(&mut buf).await.map_err(Self::io_err)?;
            if n == 0 {
                break;
            }
            let next_len = out.len().saturating_add(n);
            if next_len as u64 > max_len {
                return Err(EngineError::new(ErrorCode::InvalidArgument)
                    .with_detail(format!("remaining file size exceeds limit {}", max_len)));
            }
            out.extend_from_slice(&buf[..n]);
        }
        Ok(out)
    }

    #[allow(dead_code)] // used only with compress-brotli feature
    fn read_to_end_limited<R: std::io::Read>(
        reader: &mut R,
        max_len: u64,
        context: &str,
    ) -> Result<Vec<u8>, EngineError> {
        let mut out = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = reader.read(&mut buf).map_err(Self::io_err)?;
            if n == 0 {
                break;
            }
            let next_len = out.len().saturating_add(n);
            if next_len as u64 > max_len {
                return Err(EngineError::new(ErrorCode::InvalidArgument)
                    .with_detail(format!("{context} exceeds limit {}", max_len)));
            }
            out.extend_from_slice(&buf[..n]);
        }
        Ok(out)
    }

    fn read_zip_entry_limited<R: std::io::Read>(
        reader: &mut R,
        total_size: u64,
        position: Option<u64>,
        length: Option<u64>,
    ) -> Result<Vec<u8>, EngineError> {
        let start = position.unwrap_or(0).min(total_size);
        if let Some(len) = length {
            if len > MAX_READ_LENGTH {
                return Err(EngineError::new(ErrorCode::InvalidArgument)
                    .with_detail(format!("read length {} exceeds limit {}", len, MAX_READ_LENGTH)));
            }
        }
        let effective = length.unwrap_or_else(|| total_size.saturating_sub(start));
        if effective > MAX_READ_LENGTH {
            return Err(EngineError::new(ErrorCode::InvalidArgument)
                .with_detail(format!("zip entry size {} exceeds limit {}", effective, MAX_READ_LENGTH)));
        }

        let mut skipped = 0u64;
        let mut scratch = [0u8; 8192];
        while skipped < start {
            let want = (start - skipped).min(scratch.len() as u64) as usize;
            let n = reader.read(&mut scratch[..want]).map_err(Self::io_err)?;
            if n == 0 {
                break;
            }
            skipped += n as u64;
        }

        let mut out = Vec::new();
        while (out.len() as u64) < effective {
            let want = (effective - out.len() as u64).min(scratch.len() as u64) as usize;
            let n = reader.read(&mut scratch[..want]).map_err(Self::io_err)?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&scratch[..n]);
        }
        Ok(out)
    }

    /// Compute file size + digest in a single pass (streaming, 8 KB buffer).
    fn get_file_info(path: &str, algorithm: &str) -> Result<(u64, String), EngineError> {
        use digest::Digest;
        use std::io::Read;

        let meta = std::fs::metadata(path).map_err(Self::io_err)?;
        let size = meta.len();

        // For small files, read all at once and delegate to compute_digest.
        // For large files, stream to avoid loading everything into memory.
        if size <= 4 * 1024 * 1024 {
            let data = std::fs::read(path).map_err(Self::io_err)?;
            let digest_hex = Self::compute_digest(&data, algorithm)?;
            return Ok((size, digest_hex));
        }

        let mut file = std::io::BufReader::new(std::fs::File::open(path).map_err(Self::io_err)?);
        let mut buf = [0u8; 8192];

        macro_rules! hash_loop {
            ($hasher:expr) => {{
                let mut h = $hasher;
                loop {
                    let n = file.read(&mut buf).map_err(Self::io_err)?;
                    if n == 0 {
                        break;
                    }
                    h.update(&buf[..n]);
                }
                hex::encode(h.finalize())
            }};
        }

        let digest_hex = match algorithm {
            "md5" => hash_loop!(md5::Md5::new()),
            "sha1" => hash_loop!(sha1::Sha1::new()),
            "sha256" => hash_loop!(sha2::Sha256::new()),
            _ => {
                return Err(EngineError::new(ErrorCode::InvalidArgument)
                    .with_detail(format!("unsupported digestAlgorithm: {algorithm}")))
            }
        };

        Ok((size, digest_hex))
    }

    /// Compute digest hex string from in-memory bytes.
    fn compute_digest(data: &[u8], algorithm: &str) -> Result<String, EngineError> {
        use digest::Digest;
        match algorithm {
            "md5" => Ok(hex::encode(md5::Md5::digest(data))),
            "sha1" => Ok(hex::encode(sha1::Sha1::digest(data))),
            "sha256" => Ok(hex::encode(sha2::Sha256::digest(data))),
            _ => Err(EngineError::new(ErrorCode::InvalidArgument)
                .with_detail(format!("unsupported digestAlgorithm: {algorithm}"))),
        }
    }

    /// Compute file info from pre-read bytes (pack-backed sources).
    fn get_file_info_from_bytes(data: &[u8], algorithm: &str) -> Result<(u64, String), EngineError> {
        let digest_hex = Self::compute_digest(data, algorithm)?;
        Ok((data.len() as u64, digest_hex))
    }

    #[cfg(feature = "zip-extract")]
    fn read_zip_entries(
        zip_path: &str,
        entries_json: &str,
    ) -> Result<Vec<ZipEntryResult>, EngineError> {
        let file = std::fs::File::open(zip_path).map_err(Self::io_err)?;
        Self::read_zip_entries_from_reader(std::io::BufReader::new(file), entries_json)
    }

    #[cfg(feature = "zip-extract")]
    fn read_zip_entries_from_reader<R: std::io::Read + std::io::Seek>(
        reader: R,
        entries_json: &str,
    ) -> Result<Vec<ZipEntryResult>, EngineError> {
        use deno_core::serde_json;

        let mut archive = zip::ZipArchive::new(reader).map_err(|e| {
            EngineError::new(ErrorCode::IoError).with_detail(format!("invalid zip: {}", e))
        })?;

        let req: serde_json::Value = serde_json::from_str(entries_json).map_err(|e| {
            EngineError::new(ErrorCode::InvalidArgument)
                .with_detail(format!("invalid entries_json: {}", e))
        })?;

        let global_encoding = req
            .get("encoding")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let entries_val = req.get("entries");

        let read_all = entries_val
            .and_then(|v| v.as_str())
            .map(|s| s == "all")
            .unwrap_or(false);

        let mut results = Vec::new();

        if read_all {
            for i in 0..archive.len() {
                let mut entry = match archive.by_index(i) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                if entry.is_dir() {
                    continue;
                }
                let name = entry.name().to_string();
                let entry_size = entry.size();
                match Self::read_zip_entry_limited(&mut entry, entry_size, None, None) {
                    Ok(buf) => {
                        let data = Self::encode_zip_data(&buf, global_encoding.as_deref());
                        results.push(ZipEntryResult {
                            path: name,
                            data: Some(data),
                            err_msg: String::new(),
                        });
                    }
                    Err(e) => {
                        results.push(ZipEntryResult {
                            path: name,
                            data: None,
                            err_msg: e.to_string(),
                        });
                    }
                }
            }
        } else if let Some(arr) = entries_val.and_then(|v| v.as_array()) {
            for item in arr {
                let path = match item.get("path").and_then(|v| v.as_str()) {
                    Some(p) => p.to_string(),
                    None => continue,
                };
                let encoding = item
                    .get("encoding")
                    .and_then(|v: &serde_json::Value| v.as_str())
                    .or(global_encoding.as_deref());
                let position =
                    item.get("position").and_then(|v: &serde_json::Value| v.as_u64());
                let length = item.get("length").and_then(|v: &serde_json::Value| v.as_u64());

                match archive.by_name(&path) {
                    Ok(mut entry) => {
                        let entry_size = entry.size();
                        match Self::read_zip_entry_limited(&mut entry, entry_size, position, length) {
                            Ok(buf) => {
                                let data = Self::encode_zip_data(&buf, encoding);
                                results.push(ZipEntryResult {
                                    path,
                                    data: Some(data),
                                    err_msg: String::new(),
                                });
                            }
                            Err(e) => {
                                results.push(ZipEntryResult {
                                    path,
                                    data: None,
                                    err_msg: e.to_string(),
                                });
                            }
                        }
                    }
                    Err(e) => {
                        results.push(ZipEntryResult {
                            path,
                            data: None,
                            err_msg: format!("entry not found: {}", e),
                        });
                    }
                }
            }
        }

        Ok(results)
    }

    #[cfg(feature = "zip-extract")]
    fn encode_zip_data(data: &[u8], encoding: Option<&str>) -> String {
        use base64::Engine;
        match encoding {
            // No encoding → binary, return base64 for transport
            None => base64::engine::general_purpose::STANDARD.encode(data),
            Some(enc) => {
                // Delegate to codec for full encoding coverage (utf8, utf16le, ucs2, etc.)
                match shared::codec::decode_bytes(data, enc) {
                    Ok(s) => s,
                    // If codec doesn't support it, fall back to base64
                    Err(_) => base64::engine::general_purpose::STANDARD.encode(data),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::IoCmdHandler;
    use shared::protocol::io_cmd::MAX_READ_LENGTH;
    use std::io::Cursor;

    #[test]
    fn read_to_end_limited_rejects_oversized_output() {
        let mut reader = Cursor::new(vec![1u8; 16]);
        let err = IoCmdHandler::read_to_end_limited(&mut reader, 8, "test size").unwrap_err();
        assert!(err.to_string().contains("exceeds limit"));
    }

    #[test]
    fn read_zip_entry_limited_respects_remaining_size() {
        let mut reader = Cursor::new(vec![7u8; 32]);
        let err = IoCmdHandler::read_zip_entry_limited(
            &mut reader,
            MAX_READ_LENGTH + 16,
            Some(4),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("exceeds limit"));

        let mut ok_reader = Cursor::new(vec![9u8; 16]);
        let data = IoCmdHandler::read_zip_entry_limited(&mut ok_reader, 16, Some(8), None).unwrap();
        assert_eq!(data, vec![9u8; 8]);
    }

    #[test]
    fn read_zip_entry_limited_rejects_explicit_oversized_length() {
        let mut reader = Cursor::new(vec![0u8; 8]);
        let err = IoCmdHandler::read_zip_entry_limited(&mut reader, 8, Some(0), Some(MAX_READ_LENGTH + 1))
            .unwrap_err();
        assert!(err.to_string().contains("read length"));
    }
}

// ---------------------------------------------------------------------------
// Variant selection tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod variant_tests {
    use super::*;
    use shared::device::gpu_caps::GpuCapsSnapshot;
    use shared::protocol::io_cmd::DecodedImage;
    use std::path::PathBuf;

    /// Build a minimal valid KTX2 file with ETC2 RGB (vk_format=147).
    fn make_ktx2_etc2(width: u32, height: u32) -> Vec<u8> {
        // KTX2 header: 80 bytes + level index entry: 24 bytes + payload
        let payload = vec![0xAA; 64]; // fake compressed blocks
        let total_header = 80 + 24;
        let mut buf = Vec::with_capacity(total_header + payload.len());
        // Magic (12 bytes)
        buf.extend_from_slice(&[0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A]);
        // vkFormat = 147 (ETC2_R8G8B8)
        buf.extend_from_slice(&147u32.to_le_bytes());
        // typeSize = 1
        buf.extend_from_slice(&1u32.to_le_bytes());
        // pixelWidth
        buf.extend_from_slice(&width.to_le_bytes());
        // pixelHeight
        buf.extend_from_slice(&height.to_le_bytes());
        // pixelDepth = 0
        buf.extend_from_slice(&0u32.to_le_bytes());
        // layerCount = 0
        buf.extend_from_slice(&0u32.to_le_bytes());
        // faceCount = 1
        buf.extend_from_slice(&1u32.to_le_bytes());
        // levelCount = 1
        buf.extend_from_slice(&1u32.to_le_bytes());
        // supercompressionScheme = 0 (none)
        buf.extend_from_slice(&0u32.to_le_bytes());
        // dfdByteOffset, dfdByteLength, kvdByteOffset, kvdByteLength = 0
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        // sgdByteOffset, sgdByteLength = 0 (u64)
        buf.extend_from_slice(&0u64.to_le_bytes());
        buf.extend_from_slice(&0u64.to_le_bytes());
        assert_eq!(buf.len(), 80);
        // Level index: byteOffset(u64) + byteLength(u64) + uncompressedByteLength(u64)
        buf.extend_from_slice(&(total_header as u64).to_le_bytes());
        buf.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        buf.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        assert_eq!(buf.len(), total_header);
        buf.extend_from_slice(&payload);
        buf
    }

    /// 1x1 red PNG (smallest valid PNG).
    fn make_tiny_png() -> Vec<u8> {
        // Pre-baked 1x1 red pixel PNG.
        let png_data: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG magic
            0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
            0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1
            0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, // 8-bit RGB
            0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, // IDAT chunk
            0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, // zlib data
            0x00, 0x00, 0x04, 0x00, 0x01, 0x3B, 0xA3, 0x56, // ...
            0x8E, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, // IEND chunk
            0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        png_data.to_vec()
    }

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("migo_variant_{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn gpu_with_etc2() -> GpuCapsSnapshot {
        GpuCapsSnapshot { etc2: true, astc: false }
    }

    fn gpu_no_compressed() -> GpuCapsSnapshot {
        GpuCapsSnapshot { etc2: false, astc: false }
    }

    // ── 1. full-size + GPU supports compressed ──

    #[test]
    fn fullsize_gpu_supports_compressed_returns_compressed() {
        let dir = tmp_dir("fs_compressed");
        let ktx2_path = dir.join("tex.ktx2");
        std::fs::write(&ktx2_path, make_ktx2_etc2(4, 4)).unwrap();

        let (result, variant_kind, cache_fmt) = select_and_decode(
            ktx2_path.to_str().unwrap(),
            std::fs::read(&ktx2_path).unwrap(),
            &gpu_with_etc2(), false, None, None, None,
        ).unwrap();

        assert!(matches!(result, DecodedImage::Compressed(ref c) if c.vk_format == 147));
        assert_eq!(variant_kind, VARIANT_PRIMARY_COMPRESSED);
        assert_eq!(cache_fmt, 147);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 2. full-size + GPU does NOT support → auto fallback ──

    #[test]
    fn fullsize_gpu_no_support_auto_fallback_to_png() {
        let dir = tmp_dir("fs_fallback");
        let ktx2_path = dir.join("tex.ktx2");
        let png_path = dir.join("tex.png");
        std::fs::write(&ktx2_path, make_ktx2_etc2(4, 4)).unwrap();
        std::fs::write(&png_path, make_tiny_png()).unwrap();

        let (result, variant_kind, cache_fmt) = select_and_decode(
            ktx2_path.to_str().unwrap(),
            std::fs::read(&ktx2_path).unwrap(),
            &gpu_no_compressed(), false, None, None, None,
        ).unwrap();

        // Must return RGBA from fallback PNG, not error.
        assert!(matches!(result, DecodedImage::Rgba(ref img) if img.width == 1 && img.height == 1));
        assert_eq!(variant_kind, VARIANT_FALLBACK_RGBA);
        assert_eq!(cache_fmt, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 3. resize + compressed source → auto fallback + resize ──

    #[test]
    fn resize_with_ktx2_source_uses_fallback_png() {
        let dir = tmp_dir("resize_fb");
        let ktx2_path = dir.join("tex.ktx2");
        let png_path = dir.join("tex.png");
        std::fs::write(&ktx2_path, make_ktx2_etc2(4, 4)).unwrap();
        std::fs::write(&png_path, make_tiny_png()).unwrap();

        // Even with GPU support, resize forces fallback to RGBA.
        let (result, _, _) = select_and_decode(
            ktx2_path.to_str().unwrap(),
            std::fs::read(&ktx2_path).unwrap(),
            &gpu_with_etc2(),
            true, Some(1), Some(1), None,
        ).unwrap();

        assert!(matches!(result, DecodedImage::Rgba(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 4. preload uses same variant selection as normal load ──

    #[test]
    fn preload_and_normal_same_variant_selection() {
        let dir = tmp_dir("preload_same");
        let ktx2_path = dir.join("tex.ktx2");
        let png_path = dir.join("tex.png");
        std::fs::write(&ktx2_path, make_ktx2_etc2(4, 4)).unwrap();
        std::fs::write(&png_path, make_tiny_png()).unwrap();

        let caps = gpu_no_compressed();
        let data = std::fs::read(&ktx2_path).unwrap();
        let path = ktx2_path.to_str().unwrap();

        // Both normal load (has_resize=false) and preload (has_resize=false)
        // use the same select_and_decode → should get same variant.
        let (normal, _, _) = select_and_decode(path, data.clone(), &caps, false, None, None, None).unwrap();
        let (preload, _, _) = select_and_decode(path, data, &caps, false, None, None, None).unwrap();

        // Both should be RGBA fallback.
        assert!(matches!(normal, DecodedImage::Rgba(_)));
        assert!(matches!(preload, DecodedImage::Rgba(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 5. no fallback available → clear error ──

    #[test]
    fn ktx2_no_fallback_returns_clear_error() {
        let dir = tmp_dir("no_fallback");
        let ktx2_path = dir.join("tex.ktx2");
        std::fs::write(&ktx2_path, make_ktx2_etc2(4, 4)).unwrap();
        // No companion PNG/JPEG.

        let err = select_and_decode(
            ktx2_path.to_str().unwrap(),
            std::fs::read(&ktx2_path).unwrap(),
            &gpu_no_compressed(), false, None, None, None,
        ).unwrap_err();

        let msg = format!("{:?}", err);
        assert!(msg.contains("no") && msg.contains("fallback"), "error should mention no fallback: {msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 6. derived cache: compressed vs RGBA variants don't collide ──

    #[test]
    fn derived_cache_keys_dont_collide() {
        let dir = tmp_dir("cache_keys");
        let ktx2_path = dir.join("tex.ktx2");
        let png_path = dir.join("tex.png");
        let ktx2_data = make_ktx2_etc2(4, 4);
        let png_data = make_tiny_png();
        std::fs::write(&ktx2_path, &ktx2_data).unwrap();
        std::fs::write(&png_path, &png_data).unwrap();

        let path_str = ktx2_path.to_str().unwrap();

        // GPU supports → compressed → cache_fmt = 147
        let (compressed, compressed_kind, compressed_fmt) = select_and_decode(
            path_str,
            ktx2_data.clone(),
            &gpu_with_etc2(), false, None, None, None,
        ).unwrap();
        let (fallback, fallback_kind, fallback_fmt) = select_and_decode(
            path_str,
            ktx2_data,
            &gpu_no_compressed(), false, None, None, None,
        ).unwrap();

        assert!(matches!(compressed, DecodedImage::Compressed(_)));
        assert!(matches!(fallback, DecodedImage::Rgba(_)));
        assert_eq!(compressed_fmt, 147);
        assert_eq!(fallback_fmt, 0);
        assert_ne!(compressed_kind, fallback_kind);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 7. compressed upgrade: PNG with KTX2 companion ──

    #[test]
    fn png_with_ktx2_companion_uses_compressed_upgrade() {
        let dir = tmp_dir("upgrade");
        let png_path = dir.join("tex.png");
        let ktx2_path = dir.join("tex.ktx2");
        std::fs::write(&png_path, make_tiny_png()).unwrap();
        std::fs::write(&ktx2_path, make_ktx2_etc2(4, 4)).unwrap();

        let (result, variant_kind, cache_fmt) = select_and_decode(
            png_path.to_str().unwrap(),
            std::fs::read(&png_path).unwrap(),
            &gpu_with_etc2(), false, None, None, None,
        ).unwrap();

        // Should use compressed KTX2 companion, not decode the PNG.
        assert!(matches!(result, DecodedImage::Compressed(ref c) if c.vk_format == 147));
        assert_eq!(variant_kind, VARIANT_COMPANION_COMPRESSED);
        assert_eq!(cache_fmt, 147);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 8. PNG without KTX2 companion → standard RGBA ──

    #[test]
    fn png_without_companion_uses_rgba() {
        let dir = tmp_dir("no_upgrade");
        let png_path = dir.join("tex.png");
        std::fs::write(&png_path, make_tiny_png()).unwrap();
        // No KTX2 companion.

        let (result, variant_kind, cache_fmt) = select_and_decode(
            png_path.to_str().unwrap(),
            std::fs::read(&png_path).unwrap(),
            &gpu_with_etc2(), false, None, None, None,
        ).unwrap();

        assert!(matches!(result, DecodedImage::Rgba(_)));
        assert_eq!(variant_kind, VARIANT_PRIMARY_RGBA);
        assert_eq!(cache_fmt, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 9. resize + PNG with KTX2 companion → RGBA (no compressed for resize) ──

    #[test]
    fn resize_skips_compressed_upgrade() {
        let dir = tmp_dir("resize_skip_upgrade");
        let png_path = dir.join("tex.png");
        let ktx2_path = dir.join("tex.ktx2");
        std::fs::write(&png_path, make_tiny_png()).unwrap();
        std::fs::write(&ktx2_path, make_ktx2_etc2(4, 4)).unwrap();

        let (result, variant_kind, cache_fmt) = select_and_decode(
            png_path.to_str().unwrap(),
            std::fs::read(&png_path).unwrap(),
            &gpu_with_etc2(),
            true, Some(1), Some(1), None,
        ).unwrap();

        // Resize requested → must be RGBA, not compressed.
        assert!(matches!(result, DecodedImage::Rgba(_)));
        assert_eq!(variant_kind, VARIANT_PRIMARY_RGBA);
        assert_eq!(cache_fmt, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 10. pack-backed KTX2 with no GPU support → error (no fs fallback) ──

    #[test]
    fn ktx2_no_gpu_no_fallback_on_nonexistent_path() {
        // Path has no filesystem companions and no mount table → error.
        let err = select_and_decode(
            "/nonexistent/tex.ktx2",
            make_ktx2_etc2(4, 4),
            &gpu_no_compressed(),
            false, None, None,
            None, // no mount table
        ).unwrap_err();

        let msg = format!("{:?}", err);
        assert!(msg.contains("fallback"), "should mention fallback: {msg}");
    }

    #[test]
    fn extensionless_png_can_upgrade_to_ktx2_companion() {
        let dir = tmp_dir("extless_upgrade");
        let png_path = dir.join("tex");
        let ktx2_path = dir.join("tex.ktx2");
        std::fs::write(&png_path, make_tiny_png()).unwrap();
        std::fs::write(&ktx2_path, make_ktx2_etc2(4, 4)).unwrap();

        let (result, variant_kind, cache_fmt) = select_and_decode(
            png_path.to_str().unwrap(),
            std::fs::read(&png_path).unwrap(),
            &gpu_with_etc2(), false, None, None, None,
        ).unwrap();

        assert!(matches!(result, DecodedImage::Compressed(_)));
        assert_eq!(variant_kind, VARIANT_COMPANION_COMPRESSED);
        assert_eq!(cache_fmt, 147);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
