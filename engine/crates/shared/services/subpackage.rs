use crate::protocol::error::ServiceError;

/// Subpackage download service.
///
/// Handles downloading subpackages from the host app's CDN.
/// The JS layer resolves subpackage names to root paths via `game.json`,
/// then delegates the actual download to the platform.
///
/// JSON fields passed to both methods:
/// - `requestId`: number -- unique ID for correlating progress/result callbacks
/// - `name`: string -- subpackage name (or `"__GAME__"` for the main package)
/// - `root`: string -- normalized root path relative to code_dir (e.g. `"subpackages/stage1"`)
///
/// The platform should:
/// 1. Download the subpackage to `code_dir/{root}/`
/// 2. Report progress via `NativeMethods.onSubpackageProgress(sessionId, json)`
///    where json = `{"requestId":N,"progress":50,"totalBytesWritten":1024,"totalBytesExpectedToWrite":2048}`
/// 3. Report result via `NativeMethods.onSubpackageResult(sessionId, json)`
///    where json = `{"requestId":N}` on success or `{"requestId":N,"error":"reason"}` on failure
pub trait SubpackageService: Send + Sync {
    /// Trigger a subpackage download.
    ///
    /// Called by both `loadSubpackage` and `preDownloadSubpackage`.
    /// The JS layer handles code execution after download completes.
    fn download_subpackage(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("loadSubpackage:fail not supported"))
    }
}
