//! Game log reporting service.

use crate::protocol::error::ServiceError;

/// Game log reporting service for analytics.
///
/// Receives merged log entries from JS and forwards them to the platform.
/// The host app provides the actual log handling implementation
/// (upload to backend, forward to analytics SDK, etc.).
pub trait GameLogService: Send + Sync {
    /// Report a game log entry (JSON-encoded).
    ///
    /// The JSON is already merged with commonInfo by JS layer.
    /// Fields:
    /// - `level`: "info" | "warn" | "error" | "debug"
    /// - `key`: log tag/category string (default "default")
    /// - `value`: log content (any JSON-serializable value)
    /// - `commonInfo`: global info object at the time of reporting
    fn report_log(&self, _log_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("gameLog.log:fail not supported"))
    }
}
