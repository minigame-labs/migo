//! Clipboard service trait.

/// Clipboard service for reading/writing system clipboard.
pub trait ClipboardService: Send + Sync {
    /// Set clipboard content. Shows toast "内容已复制" on success.
    fn set_data(&self, _data: &str) -> Result<(), String> {
        Err("setClipboardData:fail not supported".to_string())
    }

    /// Get clipboard content.
    fn get_data(&self) -> Result<String, String> {
        Err("getClipboardData:fail not supported".to_string())
    }
}
