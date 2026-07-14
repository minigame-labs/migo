//! Native zip extraction using the `zip` crate.
//!
//! This provides high-performance, cross-platform zip extraction with:
//! - Path traversal protection
//! - Progress callbacks
//! - Streaming extraction (low memory usage)
//! - Resource budget (entry count, total uncompressed bytes, per-entry size,
//!   compression ratio) to defend against zip bombs

use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::{debug, error, trace, warn};
use zip::ZipArchive;

use crate::{
    pools::PoolError,
    scheduler::IoScheduler,
    task::{BackendKind, IoRequest, PriorityClass},
};

/// Resource limits applied during zip extraction / package ingest.
///
/// Path safety alone is not enough: an attacker can still craft a small
/// archive that unpacks into gigabytes of data (zip bomb), creates
/// hundreds of thousands of tiny entries (inode bomb), or repeatedly
/// inflates the same chunk (high-ratio bomb). Every extraction path
/// must check every axis **twice**: once cheaply against advertised
/// header sizes, and once against the actually-written bytes while
/// streaming.
#[derive(Debug, Clone, Copy)]
pub struct ExtractBudget {
    /// Maximum number of entries (files + directories).
    pub max_entries: usize,
    /// Maximum sum of uncompressed bytes across all entries.
    pub max_total_uncompressed: u64,
    /// Maximum uncompressed bytes for a single entry.
    pub max_entry_uncompressed: u64,
    /// Maximum `uncompressed / compressed` ratio per entry. A value
    /// of `0` disables the check (useful for `Stored` entries).
    pub max_compression_ratio: u64,
}

impl ExtractBudget {
    /// Default budget used by the engine:
    ///
    /// - 20 000 entries: covers realistic subpackages, rejects inode bombs.
    /// - 256 MiB total: covers realistic game assets without letting one
    ///   archive swallow user data.
    /// - 100 MiB per entry: matches `MAX_READ_LENGTH` elsewhere in the
    ///   engine so no single entry can exceed what the rest of the IO
    ///   stack is willing to read anyway.
    /// - 200× compression ratio: deflate on realistic game assets is
    ///   typically 3–10×; 200× is an order-of-magnitude safety net that
    ///   still rejects adversarial inflate bombs.
    pub const DEFAULT: Self = Self {
        max_entries: 20_000,
        max_total_uncompressed: 256 * 1024 * 1024,
        max_entry_uncompressed: 100 * 1024 * 1024,
        max_compression_ratio: 200,
    };
}

impl Default for ExtractBudget {
    fn default() -> Self {
        Self::DEFAULT
    }
}

fn unzip_request_for(zip_path: &Path) -> IoRequest {
    let compressed_bytes = std::fs::metadata(zip_path)
        .map(|meta| meta.len() as usize)
        .unwrap_or(0);
    IoRequest::Unzip {
        backend: BackendKind::Archive,
        priority: PriorityClass::Background,
        compressed_bytes,
    }
}

/// Error type for zip operations
#[derive(Debug)]
pub enum ZipError {
    /// The zip file was not found
    NotFound(String),
    /// IO error during extraction
    Io(io::Error),
    /// Invalid zip archive
    InvalidArchive(String),
    /// Path traversal attempt detected (security)
    PathTraversal(String),
    /// Failed to create directory
    CreateDirFailed(String),
    /// Extraction exceeded the configured budget (zip bomb defense)
    BudgetExceeded(String),
}

impl std::fmt::Display for ZipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZipError::NotFound(path) => write!(f, "Zip file not found: {}", path),
            ZipError::Io(e) => write!(f, "IO error: {}", e),
            ZipError::InvalidArchive(msg) => write!(f, "Invalid archive: {}", msg),
            ZipError::PathTraversal(path) => write!(f, "Path traversal detected: {}", path),
            ZipError::CreateDirFailed(path) => write!(f, "Failed to create directory: {}", path),
            ZipError::BudgetExceeded(msg) => write!(f, "Extraction budget exceeded: {}", msg),
        }
    }
}

impl std::error::Error for ZipError {}

impl From<io::Error> for ZipError {
    fn from(e: io::Error) -> Self {
        ZipError::Io(e)
    }
}

impl From<zip::result::ZipError> for ZipError {
    fn from(e: zip::result::ZipError) -> Self {
        ZipError::InvalidArchive(e.to_string())
    }
}

impl From<PoolError> for ZipError {
    fn from(err: PoolError) -> Self {
        match err {
            PoolError::Closed => ZipError::Io(io::Error::other("IO worker pool closed")),
        }
    }
}

/// Progress callback type
pub type ProgressCallback = Box<dyn Fn(f32, usize, usize) + Send>;

/// Reader wrapper that caps the number of bytes that can be read from
/// the underlying stream. Used for streaming decompression so a single
/// entry cannot exceed `max_entry_uncompressed` even if its zip header
/// lied about its size.
struct LimitedEntryReader<'a, R: Read> {
    inner: &'a mut R,
    remaining: u64,
}

impl<'a, R: Read> LimitedEntryReader<'a, R> {
    fn new(inner: &'a mut R, limit: u64) -> Self {
        Self {
            inner,
            remaining: limit,
        }
    }
}

impl<'a, R: Read> Read for LimitedEntryReader<'a, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "entry size exceeds per-entry budget",
            ));
        }
        let max = std::cmp::min(buf.len() as u64, self.remaining) as usize;
        let n = self.inner.read(&mut buf[..max])?;
        self.remaining -= n as u64;
        Ok(n)
    }
}

/// Extract a zip file to the destination directory with the default budget.
///
/// See [`extract_zip_with_budget`] for the full-featured version.
pub fn extract_zip(
    zip_path: &Path,
    dest_dir: &Path,
    progress: Option<ProgressCallback>,
) -> Result<(), ZipError> {
    extract_zip_with_budget(zip_path, dest_dir, progress, ExtractBudget::default())
}

/// Extract a zip file to the destination directory, enforcing a resource
/// budget against zip-bomb-style DoS inputs.
///
/// # Security
/// - Path traversal (`..`, absolute paths, symlink entries) is rejected.
/// - Ancestor directory symlinks are rejected (TOCTOU hardening).
/// - Advertised sizes are budget-checked **before** inflate; the
///   streaming decompressor is also bounded so a lying header cannot
///   bypass the per-entry cap.
/// - A running total of actually-written bytes enforces
///   `max_total_uncompressed`.
pub fn extract_zip_with_budget(
    zip_path: &Path,
    dest_dir: &Path,
    progress: Option<ProgressCallback>,
    budget: ExtractBudget,
) -> Result<(), ZipError> {
    debug!(
        "extract_zip: zip={} dest={} budget={:?}",
        zip_path.display(),
        dest_dir.display(),
        budget
    );

    if !zip_path.exists() {
        return Err(ZipError::NotFound(zip_path.display().to_string()));
    }

    let file = File::open(zip_path)?;
    let reader = BufReader::with_capacity(64 * 1024, file);
    let mut archive = ZipArchive::new(reader)?;

    let total_files = archive.len();
    debug!("extract_zip: {} files in archive", total_files);

    if total_files > budget.max_entries {
        return Err(ZipError::BudgetExceeded(format!(
            "entry count {} exceeds limit {}",
            total_files, budget.max_entries
        )));
    }

    // Cheap header-time pre-scan. We pay a single metadata pass so we
    // can reject obviously bomb-shaped archives without touching inflate.
    let mut advertised_total: u64 = 0;
    for i in 0..total_files {
        let entry = archive.by_index_raw(i)?;
        let advertised = entry.size();
        if advertised > budget.max_entry_uncompressed {
            return Err(ZipError::BudgetExceeded(format!(
                "entry '{}' advertises {} bytes, exceeds per-entry limit {}",
                entry.name(),
                advertised,
                budget.max_entry_uncompressed
            )));
        }
        advertised_total = advertised_total.saturating_add(advertised);
        if advertised_total > budget.max_total_uncompressed {
            return Err(ZipError::BudgetExceeded(format!(
                "advertised total {} bytes exceeds limit {}",
                advertised_total, budget.max_total_uncompressed
            )));
        }
    }

    fs::create_dir_all(dest_dir)?;
    let dest_canonical = dest_dir.canonicalize()?;

    let mut written_total: u64 = 0;

    for i in 0..total_files {
        let mut file = archive.by_index(i)?;
        let file_name = file.name().to_string();

        trace!("extract_zip: processing [{}] {}", i, file_name);

        let outpath = dest_dir.join(&file_name);

        if file.is_symlink() {
            error!("extract_zip: symlink entry rejected: {}", file_name);
            return Err(ZipError::PathTraversal(format!(
                "symlink entry not allowed: {}",
                file_name
            )));
        }

        let outpath_normalized = normalize_path(&outpath);
        if !outpath_normalized.starts_with(&dest_canonical) {
            error!(
                "extract_zip: path traversal detected: {} -> {}",
                file_name,
                outpath_normalized.display()
            );
            return Err(ZipError::PathTraversal(file_name));
        }

        if let Some(parent) = outpath_normalized.parent() {
            if parent.exists() {
                match std::fs::canonicalize(parent) {
                    Ok(canonical_parent) => {
                        if !canonical_parent.starts_with(&dest_canonical) {
                            error!(
                                "extract_zip: ancestor symlink escape detected: {} -> {}",
                                parent.display(),
                                canonical_parent.display()
                            );
                            return Err(ZipError::PathTraversal(format!(
                                "ancestor directory escapes target via symlink: {}",
                                file_name
                            )));
                        }
                    }
                    Err(e) => {
                        error!(
                            "extract_zip: cannot canonicalize parent {}: {} — rejecting entry {}",
                            parent.display(),
                            e,
                            file_name
                        );
                        return Err(ZipError::PathTraversal(format!(
                            "cannot verify ancestor directory: {}",
                            file_name
                        )));
                    }
                }
            }
        }

        if file.is_dir() {
            trace!("extract_zip: creating directory {}", outpath.display());
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(parent) = outpath.parent() {
                if !parent.exists() {
                    trace!("extract_zip: creating parent dir {}", parent.display());
                    fs::create_dir_all(parent)?;
                }
            }

            // Per-entry ratio check: uncompressed / compressed.
            let compressed = file.compressed_size();
            let uncompressed_hdr = file.size();
            if budget.max_compression_ratio > 0 && compressed > 0 {
                let ratio = uncompressed_hdr / compressed;
                if ratio > budget.max_compression_ratio {
                    return Err(ZipError::BudgetExceeded(format!(
                        "entry '{}' compression ratio {} exceeds limit {}",
                        file_name, ratio, budget.max_compression_ratio
                    )));
                }
            }

            // Streaming copy bounded by the minimum of the per-entry
            // cap and the remaining total budget. If either is hit we
            // reject the archive instead of silently truncating.
            let remaining_total = budget.max_total_uncompressed.saturating_sub(written_total);
            let cap = std::cmp::min(budget.max_entry_uncompressed, remaining_total);
            let mut limited = LimitedEntryReader::new(&mut file, cap + 1);

            let mut outfile = File::create(&outpath)?;
            let actually_written = match io::copy(&mut limited, &mut outfile) {
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                    let _ = fs::remove_file(&outpath);
                    return Err(ZipError::BudgetExceeded(format!(
                        "entry '{}' exceeded per-entry or total budget",
                        file_name
                    )));
                }
                Err(e) => return Err(ZipError::Io(e)),
            };

            if actually_written > budget.max_entry_uncompressed {
                let _ = fs::remove_file(&outpath);
                return Err(ZipError::BudgetExceeded(format!(
                    "entry '{}' wrote {} bytes, exceeds per-entry limit {}",
                    file_name, actually_written, budget.max_entry_uncompressed
                )));
            }

            written_total = written_total.saturating_add(actually_written);
            if written_total > budget.max_total_uncompressed {
                let _ = fs::remove_file(&outpath);
                return Err(ZipError::BudgetExceeded(format!(
                    "archive wrote {} bytes, exceeds total limit {}",
                    written_total, budget.max_total_uncompressed
                )));
            }

            trace!(
                "extract_zip: extracted {} ({} bytes, total {})",
                outpath.display(),
                actually_written,
                written_total
            );

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = file.unix_mode() {
                    if let Err(e) = fs::set_permissions(&outpath, fs::Permissions::from_mode(mode))
                    {
                        warn!("extract_zip: failed to set permissions: {}", e);
                    }
                }
            }
        }

        if let Some(ref callback) = progress {
            let prog = (i + 1) as f32 / total_files as f32;
            callback(prog, i + 1, total_files);
        }
    }

    debug!(
        "extract_zip: completed, {} files extracted, {} bytes total",
        total_files, written_total
    );
    Ok(())
}

/// Normalize a path without requiring it to exist.
/// This handles .. and . components manually.
fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();

    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                result.pop();
            }
            std::path::Component::CurDir => {}
            c => {
                result.push(c);
            }
        }
    }

    result
}

/// Extract zip file asynchronously (runs in a blocking thread pool).
///
/// Uses the default `ExtractBudget`. For a custom budget see
/// [`extract_zip_with_scheduler_and_budget`].
pub async fn extract_zip_with_scheduler(
    scheduler: Arc<IoScheduler>,
    zip_path: PathBuf,
    dest_dir: PathBuf,
    progress_tx: Option<tokio::sync::mpsc::Sender<(f32, usize, usize)>>,
) -> Result<(), ZipError> {
    extract_zip_with_scheduler_and_budget(
        scheduler,
        zip_path,
        dest_dir,
        progress_tx,
        ExtractBudget::default(),
    )
    .await
}

pub async fn extract_zip_with_scheduler_and_budget(
    scheduler: Arc<IoScheduler>,
    zip_path: PathBuf,
    dest_dir: PathBuf,
    progress_tx: Option<tokio::sync::mpsc::Sender<(f32, usize, usize)>>,
    budget: ExtractBudget,
) -> Result<(), ZipError> {
    let request = unzip_request_for(&zip_path);

    scheduler
        .run_async(request, move || {
            let progress: Option<ProgressCallback> = progress_tx.map(|tx| {
                Box::new(move |prog: f32, current: usize, total: usize| {
                    let _ = tx.blocking_send((prog, current, total));
                }) as ProgressCallback
            });

            extract_zip_with_budget(&zip_path, &dest_dir, progress, budget)
        })
        .await
        .map_err(ZipError::from)?
}

pub async fn extract_zip_async(
    scheduler: Arc<IoScheduler>,
    zip_path: PathBuf,
    dest_dir: PathBuf,
    progress_tx: Option<tokio::sync::mpsc::Sender<(f32, usize, usize)>>,
) -> Result<(), ZipError> {
    extract_zip_with_scheduler(scheduler, zip_path, dest_dir, progress_tx).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Arc;

    use crate::scheduler::IoScheduler;

    #[test]
    fn test_normalize_path() {
        let p = normalize_path(Path::new("/a/b/../c/./d"));
        assert_eq!(p, PathBuf::from("/a/c/d"));

        let p = normalize_path(Path::new("a/b/../../c"));
        assert_eq!(p, PathBuf::from("c"));
    }

    #[test]
    fn test_path_traversal_detection() {
        let dest = Path::new("/tmp/dest");
        let malicious = dest.join("../../../etc/passwd");
        let normalized = normalize_path(&malicious);
        let dest_canonical = dest.to_path_buf();
        assert!(!normalized.starts_with(&dest_canonical));
    }

    #[test]
    fn unzip_requests_use_archive_pool() {
        let dir =
            std::env::temp_dir().join(format!("migo_zip_extract_scheduler_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let zip_path = dir.join("input.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("hello.txt", options).unwrap();
        zip.write_all(b"hello archive").unwrap();
        zip.finish().unwrap();

        let dest_dir = dir.join("out");
        let scheduler = Arc::new(IoScheduler::local_for_test(31, 2));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        runtime
            .block_on(extract_zip_with_scheduler(
                Arc::clone(&scheduler),
                zip_path.clone(),
                dest_dir.clone(),
                None,
            ))
            .unwrap();

        assert_eq!(scheduler.pools().started_thread_count_for_test(), 2);
        assert_eq!(
            std::fs::read(dest_dir.join("hello.txt")).unwrap(),
            b"hello archive"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unzip_requests_default_to_background_priority() {
        let request = unzip_request_for(Path::new("/tmp/archive.zip"));
        match request {
            IoRequest::Unzip { priority, .. } => {
                assert_eq!(priority, PriorityClass::Background);
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }

    #[test]
    fn extract_zip_async_uses_provided_scheduler() {
        let dir =
            std::env::temp_dir().join(format!("migo_zip_extract_async_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let zip_path = dir.join("input.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("hello.txt", options).unwrap();
        zip.write_all(b"hello archive async").unwrap();
        zip.finish().unwrap();

        let dest_dir = dir.join("out");
        let scheduler = Arc::new(IoScheduler::new(43));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        runtime
            .block_on(extract_zip_async(
                Arc::clone(&scheduler),
                zip_path.clone(),
                dest_dir.clone(),
                None,
            ))
            .unwrap();

        assert_eq!(
            std::fs::read(dest_dir.join("hello.txt")).unwrap(),
            b"hello archive async"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn budget_rejects_too_many_entries() {
        let dir =
            std::env::temp_dir().join(format!("migo_zip_budget_entries_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let zip_path = dir.join("many.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for i in 0..10 {
            zip.start_file(format!("f_{i}.txt"), options).unwrap();
            zip.write_all(b"x").unwrap();
        }
        zip.finish().unwrap();

        let dest_dir = dir.join("out");
        let budget = ExtractBudget {
            max_entries: 3,
            ..ExtractBudget::DEFAULT
        };
        let res = extract_zip_with_budget(&zip_path, &dest_dir, None, budget);
        assert!(matches!(res, Err(ZipError::BudgetExceeded(_))));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn budget_rejects_single_large_entry() {
        let dir =
            std::env::temp_dir().join(format!("migo_zip_budget_entry_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let zip_path = dir.join("big.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("big.bin", options).unwrap();
        zip.write_all(&vec![0u8; 4096]).unwrap();
        zip.finish().unwrap();

        let dest_dir = dir.join("out");
        let budget = ExtractBudget {
            max_entry_uncompressed: 1024,
            max_total_uncompressed: 1024,
            ..ExtractBudget::DEFAULT
        };
        let res = extract_zip_with_budget(&zip_path, &dest_dir, None, budget);
        assert!(matches!(res, Err(ZipError::BudgetExceeded(_))));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn budget_rejects_total_overflow() {
        let dir =
            std::env::temp_dir().join(format!("migo_zip_budget_total_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let zip_path = dir.join("total.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for i in 0..4 {
            zip.start_file(format!("f_{i}.bin"), options).unwrap();
            zip.write_all(&vec![0u8; 1024]).unwrap();
        }
        zip.finish().unwrap();

        let dest_dir = dir.join("out");
        let budget = ExtractBudget {
            max_entry_uncompressed: 4096,
            max_total_uncompressed: 2048,
            ..ExtractBudget::DEFAULT
        };
        let res = extract_zip_with_budget(&zip_path, &dest_dir, None, budget);
        assert!(matches!(res, Err(ZipError::BudgetExceeded(_))));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn budget_allows_normal_archive() {
        let dir = std::env::temp_dir().join(format!("migo_zip_budget_ok_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let zip_path = dir.join("ok.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("a.txt", options).unwrap();
        zip.write_all(b"hello").unwrap();
        zip.start_file("b.txt", options).unwrap();
        zip.write_all(b"world").unwrap();
        zip.finish().unwrap();

        let dest_dir = dir.join("out");
        extract_zip_with_budget(&zip_path, &dest_dir, None, ExtractBudget::default()).unwrap();
        assert_eq!(std::fs::read(dest_dir.join("a.txt")).unwrap(), b"hello");
        assert_eq!(std::fs::read(dest_dir.join("b.txt")).unwrap(), b"world");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
