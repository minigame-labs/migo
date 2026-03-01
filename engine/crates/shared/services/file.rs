//! File service trait for platform-specific file operations.

/// Platform-specific file service for operations not available in pure Rust.
///
/// Currently used for zip extraction:
/// - Android: Uses `java.util.zip` via JNI
/// - Desktop: Falls back to Rust `zip` crate (via `zip-extract` feature on IO crate)
pub trait FileService: Send + Sync {
    /// Extract a zip file to the target directory.
    /// Returns the number of files extracted on success.
    fn unzip(&self, zip_path: &str, dest_dir: &str) -> Result<usize, String>;
}
