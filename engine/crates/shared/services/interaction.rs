//! UI interaction service traits for toast, modal, loading, and action sheet.

use std::sync::Arc;

// ==================== Interaction ====================

/// UI interaction service for showing toasts, modals, loading indicators, and action sheets.
pub trait InteractionService: Send + Sync {
    /// Show a toast notification. `json`: `{"title","icon","duration","mask"}`
    fn show_toast(&self, _json: &str) -> Result<(), String> {
        Err("showToast:fail not supported".to_string())
    }

    /// Hide the current toast.
    fn hide_toast(&self) -> Result<(), String> {
        Err("hideToast:fail not supported".to_string())
    }

    /// Show a modal dialog. `json`: `{"title","content","showCancel","cancelText","confirmText",...}`
    fn show_modal(&self, _json: &str) -> Result<(), String> {
        Err("showModal:fail not supported".to_string())
    }

    /// Show a loading indicator. `json`: `{"title","mask"}`
    fn show_loading(&self, _json: &str) -> Result<(), String> {
        Err("showLoading:fail not supported".to_string())
    }

    /// Hide the current loading indicator.
    fn hide_loading(&self) -> Result<(), String> {
        Err("hideLoading:fail not supported".to_string())
    }

    /// Show an action sheet. `json`: `{"alertText","itemList","itemColor"}`
    fn show_action_sheet(&self, _json: &str) -> Result<(), String> {
        Err("showActionSheet:fail not supported".to_string())
    }
}
