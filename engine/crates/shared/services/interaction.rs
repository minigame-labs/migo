//! UI interaction service traits for toast, modal, loading, and action sheet.

use crate::protocol::error::ServiceError;

// ==================== Interaction ====================

/// UI interaction service for showing toasts, modals, loading indicators, and action sheets.
pub trait InteractionService: Send + Sync {
    /// Show a toast notification. `json`: `{"title","icon","duration","mask"}`
    fn show_toast(&self, _json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("showToast:fail not supported"))
    }

    /// Hide the current toast.
    fn hide_toast(&self) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("hideToast:fail not supported"))
    }

    /// Show a modal dialog. `json`: `{"title","content","showCancel","cancelText","confirmText",...}`
    fn show_modal(&self, _json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("showModal:fail not supported"))
    }

    /// Show a loading indicator. `json`: `{"title","mask"}`
    fn show_loading(&self, _json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("showLoading:fail not supported"))
    }

    /// Hide the current loading indicator.
    fn hide_loading(&self) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("hideLoading:fail not supported"))
    }

    /// Show an action sheet. `json`: `{"alertText","itemList","itemColor"}`
    fn show_action_sheet(&self, _json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("showActionSheet:fail not supported"))
    }
}
