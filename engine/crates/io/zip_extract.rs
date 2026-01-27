//! Native zip extraction using the `zip` crate.
//!
//! This provides high-performance, cross-platform zip extraction with:
//! - Path traversal protection
//! - Progress callbacks
//! - Streaming extraction (low memory usage)

use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use tracing::{debug, error, trace, warn};
use zip::ZipArchive;

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
}

impl std::fmt::Display for ZipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZipError::NotFound(path) => write!(f, "Zip file not found: {}", path),
            ZipError::Io(e) => write!(f, "IO error: {}", e),
            ZipError::InvalidArchive(msg) => write!(f, "Invalid archive: {}", msg),
            ZipError::PathTraversal(path) => write!(f, "Path traversal detected: {}", path),
            ZipError::CreateDirFailed(path) => write!(f, "Failed to create directory: {}", path),
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

/// Progress callback type
pub type ProgressCallback = Box<dyn Fn(f32, usize, usize) + Send>;

/// Extract a zip file to the destination directory.
///
/// # Arguments
/// * `zip_path` - Path to the zip file
/// * `dest_dir` - Destination directory
/// * `progress` - Optional progress callback (progress 0.0-1.0, current_file, total_files)
///
/// # Returns
/// * `Ok(())` on success
/// * `Err(ZipError)` on failure
///
/// # Security
/// This function includes path traversal protection to prevent zip slip attacks.
pub fn extract_zip(
    zip_path: &Path,
    dest_dir: &Path,
    progress: Option<ProgressCallback>,
) -> Result<(), ZipError> {
    debug!(
        "extract_zip: zip={} dest={}",
        zip_path.display(),
        dest_dir.display()
    );

    // Check if zip file exists
    if !zip_path.exists() {
        return Err(ZipError::NotFound(zip_path.display().to_string()));
    }

    // Open the zip file
    let file = File::open(zip_path)?;
    let reader = BufReader::with_capacity(64 * 1024, file);
    let mut archive = ZipArchive::new(reader)?;

    let total_files = archive.len();
    debug!("extract_zip: {} files in archive", total_files);

    // Ensure destination directory exists
    fs::create_dir_all(dest_dir)?;

    // Get canonical path for security check
    let dest_canonical = dest_dir.canonicalize()?;

    for i in 0..total_files {
        let mut file = archive.by_index(i)?;
        let file_name = file.name().to_string();

        trace!("extract_zip: processing [{}] {}", i, file_name);

        // Build output path
        let outpath = dest_dir.join(&file_name);

        // Security: check for path traversal
        // We need to handle the case where outpath doesn't exist yet
        let outpath_normalized = normalize_path(&outpath);
        if !outpath_normalized.starts_with(&dest_canonical) {
            error!(
                "extract_zip: path traversal detected: {} -> {}",
                file_name,
                outpath_normalized.display()
            );
            return Err(ZipError::PathTraversal(file_name));
        }

        if file.is_dir() {
            // Create directory
            trace!("extract_zip: creating directory {}", outpath.display());
            fs::create_dir_all(&outpath)?;
        } else {
            // Create parent directories if needed
            if let Some(parent) = outpath.parent() {
                if !parent.exists() {
                    trace!("extract_zip: creating parent dir {}", parent.display());
                    fs::create_dir_all(parent)?;
                }
            }

            // Extract file
            let mut outfile = File::create(&outpath)?;
            io::copy(&mut file, &mut outfile)?;

            trace!(
                "extract_zip: extracted {} ({} bytes)",
                outpath.display(),
                file.size()
            );

            // Set permissions on Unix
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

        // Report progress
        if let Some(ref callback) = progress {
            let prog = (i + 1) as f32 / total_files as f32;
            callback(prog, i + 1, total_files);
        }
    }

    debug!("extract_zip: completed, {} files extracted", total_files);
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
/// This is the preferred method for use with async runtimes.
pub async fn extract_zip_async(
    zip_path: PathBuf,
    dest_dir: PathBuf,
    progress_tx: Option<tokio::sync::mpsc::Sender<(f32, usize, usize)>>,
) -> Result<(), ZipError> {
    tokio::task::spawn_blocking(move || {
        let progress: Option<ProgressCallback> = progress_tx.map(|tx| {
            Box::new(move |prog: f32, current: usize, total: usize| {
                let _ = tx.blocking_send((prog, current, total));
            }) as ProgressCallback
        });

        extract_zip(&zip_path, &dest_dir, progress)
    })
    .await
    .map_err(|e| ZipError::Io(io::Error::new(io::ErrorKind::Other, e.to_string())))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path() {
        let p = normalize_path(Path::new("/a/b/../c/./d"));
        assert_eq!(p, PathBuf::from("/a/c/d"));

        let p = normalize_path(Path::new("a/b/../../c"));
        assert_eq!(p, PathBuf::from("c"));
    }

    #[test]
    fn test_path_traversal_detection() {
        // This would be caught by our normalize_path check
        let dest = Path::new("/tmp/dest");
        let malicious = dest.join("../../../etc/passwd");
        let normalized = normalize_path(&malicious);

        // The normalized path should NOT start with /tmp/dest
        let dest_canonical = dest.to_path_buf(); // Simplified for test
        assert!(!normalized.starts_with(&dest_canonical));
    }
}
