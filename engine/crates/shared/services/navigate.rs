//! Navigation service traits for mini program navigation and customer service.

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
    fn navigate_to_mini_program(&self, _options_json: &str) -> Result<(), String> {
        Err("navigateToMiniProgram:fail not supported".to_string())
    }

    /// Navigate back to the source mini program (Mode A, sync).
    ///
    /// JSON fields (input):
    /// - `extraData`: object (optional, data to pass back)
    fn navigate_back_mini_program(&self, _options_json: &str) -> Result<(), String> {
        Err("navigateBackMiniProgram:fail not supported".to_string())
    }

    /// Open the customer service conversation (Mode A, sync).
    ///
    /// JSON fields (input):
    /// - `sessionFrom`: string (optional)
    /// - `showMessageCard`: boolean (optional)
    /// - `sendMessageTitle`: string (optional)
    /// - `sendMessagePath`: string (optional)
    /// - `sendMessageImg`: string (optional)
    fn open_customer_service_conversation(&self, _options_json: &str) -> Result<(), String> {
        Err("openCustomerServiceConversation:fail not supported".to_string())
    }
}
