use std::sync::Arc;

use deno_core::{op2, OpState};
use tracing::{error, info};

use shared::{
    op_state::{CanvasOpState, HostOpState},
    protocol::{render_cmd::RenderCommand, send_render_with_resp_sync},
    vfs::FileOp,
};

const OP_LOAD_FONT: &str = "load_font";
const OP_GET_TEXT_LINE_HEIGHT: &str = "get_text_line_height";

fn resolve_font_src_path(
    code_dir: &str,
    vfs: Option<&shared::vfs::VirtualFS>,
    src: &str,
) -> Result<String, String> {
    if let Some(vfs) = vfs {
        if !src.starts_with('/') {
            let vpath = format!("/code/{src}");
            return vfs
                .resolve(&vpath, FileOp::Read)
                .map(|p| p.to_string_lossy().into_owned())
                .map_err(|e| format!("resolve vpath {} failed: {}", vpath, e));
        }

        let is_virtual = src == "/code"
            || src.starts_with("/code/")
            || src == "/user"
            || src.starts_with("/user/")
            || src == "/cache"
            || src.starts_with("/cache/")
            || src == "/tmp"
            || src.starts_with("/tmp/");

        if is_virtual {
            return vfs
                .resolve(src, FileOp::Read)
                .map(|p| p.to_string_lossy().into_owned())
                .map_err(|e| format!("resolve vpath {} failed: {}", src, e));
        }
    }

    if std::path::Path::new(src).is_absolute() {
        return Ok(src.to_string());
    }

    if code_dir.is_empty() {
        return Ok(src.to_string());
    }

    Ok(std::path::Path::new(code_dir)
        .join(src)
        .to_string_lossy()
        .into_owned())
}

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
    // Resolve font path with VFS support (/code, /user, /cache, /tmp).
    let resolved = {
        let host = state.borrow::<HostOpState>();
        let code_dir = host.code_dir.as_deref().unwrap_or("");
        match resolve_font_src_path(code_dir, host.vfs.as_deref(), &path) {
            Ok(p) => p,
            Err(e) => {
                error!("op_load_font: failed to resolve '{}': {}", path, e);
                return String::new();
            }
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

#[cfg(test)]
mod tests {
    use super::resolve_font_src_path;
    use shared::vfs::VirtualFS;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn make_temp_base() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("migo_font_vfs_test_{}", nanos))
    }

    #[test]
    fn resolves_user_virtual_path_via_vfs() {
        let base = make_temp_base();
        let code = base.join("code");
        let user = base.join("user");
        let cache = base.join("cache");
        let tmp = base.join("tmp");

        fs::create_dir_all(&code).unwrap();
        fs::create_dir_all(user.join("gamecaches/resources")).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&tmp).unwrap();

        let font_path = user.join("gamecaches/resources/test.ttf");
        fs::write(&font_path, b"font-bytes").unwrap();

        let vfs = VirtualFS::new(code, user, cache, tmp);
        let resolved =
            resolve_font_src_path("", Some(&vfs), "/user/gamecaches/resources/test.ttf").unwrap();

        assert_eq!(resolved, font_path.to_string_lossy());

        let _ = fs::remove_dir_all(base);
    }
}
