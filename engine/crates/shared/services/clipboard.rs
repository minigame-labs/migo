//! Clipboard service trait.

use crate::protocol::error::ServiceError;

/// Clipboard service for reading/writing system clipboard.
pub trait ClipboardService: Send + Sync {
    /// Set clipboard content. Shows toast on success.
    fn set_data(&self, _data: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("setClipboardData:fail not supported"))
    }

    /// Get clipboard content.
    fn get_data(&self) -> Result<String, ServiceError> {
        Err(ServiceError::not_supported("getClipboardData:fail not supported"))
    }
}
