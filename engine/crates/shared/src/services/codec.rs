//! Codec service trait for platform-specific encoding/decoding.

use crate::protocol::error::ServiceError;

/// Platform-specific codec service for encodings not available in pure Rust.
///
/// Currently used for GBK encoding/decoding:
/// - Android: Uses `java.nio.charset.Charset` via JNI
/// - Desktop: Falls back to `encoding_rs` crate (via `codec-gbk` feature)
pub trait CodecService: Send + Sync {
    /// Encode a string to GBK bytes.
    fn encode_gbk(&self, data: &str) -> Result<Vec<u8>, ServiceError>;

    /// Decode GBK bytes to a string.
    fn decode_gbk(&self, data: &[u8]) -> Result<String, ServiceError>;
}
