use std::sync::Arc;

use deno_core::{OpState, op2};
use tracing::{error, info};

use shared::{
    op_state::{CanvasOpState, HostOpState},
    protocol::{render_cmd::RenderCommand, send_render_with_resp_sync},
};

const OP_LOAD_FONT: &str = "load_font";
const OP_GET_TEXT_LINE_HEIGHT: &str = "get_text_line_height";

/// Load a custom font file and register it globally.
///
/// Resolves the path relative to the game's code directory, reads the font
/// bytes, sends them to the render thread for registration in both the global
/// font store and all existing canvas FontManagers.
///
/// Returns the font family key (file stem) on success, or empty string on failure.
#[op2]
#[string]
pub(crate) fn op_load_font(state: &mut OpState, #[string] path: String) -> String {
    // Resolve path relative to game code directory.
    let resolved = {
        let host = state.borrow::<HostOpState>();
        match &host.code_dir {
            Some(code_dir) => {
                let p = std::path::Path::new(code_dir).join(&path);
                p.to_string_lossy().to_string()
            }
            None => path.clone(),
        }
    };

    // Read font file bytes.
    let bytes = match std::fs::read(&resolved) {
        Ok(b) => b,
        Err(e) => {
            error!("op_load_font: failed to read '{}': {}", resolved, e);
            return String::new();
        }
    };

    if bytes.is_empty() {
        error!("op_load_font: font file is empty: {}", resolved);
        return String::new();
    }

    // Derive the font family key from the file stem.
    let key = std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("custom-font")
        .to_lowercase();

    let bytes = Arc::new(bytes);

    // Send to render thread for registration.
    let ctx = state.borrow::<CanvasOpState>();
    match send_render_with_resp_sync(ctx, OP_LOAD_FONT, |resp| RenderCommand::LoadFont {
        key: key.clone(),
        bytes: bytes.clone(),
        resp,
    }) {
        Ok(family) => {
            info!("op_load_font: loaded '{}' as '{}'", path, family);
            family
        }
        Err(e) => {
            error!("op_load_font: render thread error: {}", e);
            String::new()
        }
    }
}

/// Measure the line height of text with the given font configuration.
///
/// Parameters are parsed from the JS object: fontStyle, fontWeight, fontSize, fontFamily.
/// Returns the line height in pixels (ascender - descender), or fontSize * 1.2 as fallback.
#[op2(fast)]
pub(crate) fn op_get_text_line_height(
    state: &mut OpState,
    #[string] font_family: String,
    font_size: f64,
    bold: bool,
    italic: bool,
) -> f64 {
    let fs = font_size as f32;
    let ctx = state.borrow::<CanvasOpState>();
    match send_render_with_resp_sync(ctx, OP_GET_TEXT_LINE_HEIGHT, |resp| {
        RenderCommand::GetTextLineHeight {
            font_family,
            font_size: fs,
            bold,
            italic,
            resp,
        }
    }) {
        Ok(height) => height as f64,
        Err(e) => {
            error!("op_get_text_line_height: render thread error: {}", e);
            font_size * 1.2
        }
    }
}
