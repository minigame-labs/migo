//! UI interaction ops and ESM modules.
//!
//! This module provides cross-platform UI interaction APIs (toast, modal,
//! loading, action sheet) using trait-based services injected via
//! `HostOpState.device_services`.

use deno_core::{Extension, OpState, op2};
use deno_error::JsErrorBox;
use shared::op_state::HostOpState;

// ==================== Toast Ops ====================

#[op2(fast)]
pub fn op_show_toast(state: &mut OpState, #[string] json: String) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(interaction) = services.interaction() {
            return interaction.show_toast(&json).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("showToast:fail not supported"))
}

#[op2(fast)]
pub fn op_hide_toast(state: &mut OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(interaction) = services.interaction() {
            return interaction.hide_toast().map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("hideToast:fail not supported"))
}

// ==================== Modal Ops ====================

#[op2(fast)]
pub fn op_show_modal(state: &mut OpState, #[string] json: String) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(interaction) = services.interaction() {
            return interaction.show_modal(&json).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("showModal:fail not supported"))
}

// ==================== Loading Ops ====================

#[op2(fast)]
pub fn op_show_loading(state: &mut OpState, #[string] json: String) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(interaction) = services.interaction() {
            return interaction.show_loading(&json).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("showLoading:fail not supported"))
}

#[op2(fast)]
pub fn op_hide_loading(state: &mut OpState) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(interaction) = services.interaction() {
            return interaction.hide_loading().map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("hideLoading:fail not supported"))
}

// ==================== Action Sheet Ops ====================

#[op2(fast)]
pub fn op_show_action_sheet(state: &mut OpState, #[string] json: String) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(interaction) = services.interaction() {
            return interaction
                .show_action_sheet(&json)
                .map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("showActionSheet:fail not supported"))
}

// ==================== Menu Button Ops ====================

#[op2]
#[string]
pub fn op_get_menu_button_rect(state: &mut OpState) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(sys_info) = services.system_info() {
            return sys_info
                .get_menu_button_bounding_client_rect_json()
                .map_err(JsErrorBox::generic);
        }
    }
    // Fallback: default rect (same as trait default)
    Ok(r#"{"width":87,"height":32,"top":4,"bottom":36,"left":278,"right":365}"#.to_string())
}

// ==================== Extension Definition ====================

deno_core::extension!(
    host_v8_ui,
    deps = [host_v8_base],
    ops = [
        op_show_toast,
        op_hide_toast,
        op_show_modal,
        op_show_loading,
        op_hide_loading,
        op_show_action_sheet,
        op_get_menu_button_rect,
    ],
    esm = [
        dir "ui",
        "01_interaction.js",
        "02_buttons.js",
        "03_page_manager.js",
        "99_global_scope.js",
    ]
);

pub fn ui_extensions() -> Vec<Extension> {
    vec![host_v8_ui::init()]
}
