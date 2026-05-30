//! Share service traits for share-related operations.

use crate::protocol::error::ServiceError;

/// Share service for app sharing operations.
///
/// Mode C (async): `share_app_message` fires the platform share flow;
/// the result arrives via `_internalOnShareAppMessageResult` EvalScript callback.
pub trait ShareService: Send + Sync {
    /// Trigger the native share flow.
    ///
    /// JSON fields (input):
    /// - `title`: string
    /// - `imageUrl`: string
    /// - `query`: string
    /// - `imageUrlId`: string (optional)
    ///
    /// Result delivered via `onShareAppMessageResult` callback.
    fn share_app_message(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported(
            "shareAppMessage:fail not supported",
        ))
    }
}
