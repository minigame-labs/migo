//! Navigation service traits for mini program navigation and customer service.

use crate::protocol::error::ServiceError;

/// Navigation service for cross-app and customer service operations.
pub trait NavigateService: Send + Sync {
    /// Navigate to another mini program (Mode C, async).
    ///
    /// JSON fields (input):
    /// - `appId`: string (required)
    /// - `path`: string (optional)
    /// - `extraData`: object (optional)
    /// - `envVersion`: string (optional, "develop"|"trial"|"release")
    ///
    /// Result delivered via `onNavigateToMiniProgramResult` callback.
    fn navigate_to_mini_program(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("navigateToMiniProgram:fail not supported"))
    }

    /// Navigate back to the source mini program (Mode A, sync).
    ///
    /// JSON fields (input):
    /// - `extraData`: object (optional, data to pass back)
    fn navigate_back_mini_program(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("navigateBackMiniProgram:fail not supported"))
    }

    /// Open the customer service conversation (Mode A, sync).
    ///
    /// JSON fields (input):
    /// - `sessionFrom`: string (optional)
    /// - `showMessageCard`: boolean (optional)
    /// - `sendMessageTitle`: string (optional)
    /// - `sendMessagePath`: string (optional)
    /// - `sendMessageImg`: string (optional)
    fn open_customer_service_conversation(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("openCustomerServiceConversation:fail not supported"))
    }
}
