use std::sync::Arc;

use deno_core::{OpState, op2};
use tracing::{error, info};

use shared::{
    op_state::{CanvasOpState, HostOpState},
    protocol::{render_cmd::RenderCommand, send_render_with_resp_sync},
    vfs::FileOp,
};

const OP_LOAD_FONT: &str = "load_font";
const OP_GET_TEXT_LINE_HEIGHT: &str = "get_text_line_height";

#[derive(Debug, PartialEq, Eq)]
struct FontRegistrationRequest {
    family: String,
    aliases: Vec<String>,
}

fn normalize_family_name(candidate: &str) -> Option<String> {
    let trimmed = candidate
        .trim()
        .trim_matches(|ch| ch == '"' || ch == '\'')
        .trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn push_alias(aliases: &mut Vec<String>, alias: String) {
    let key = alias.to_lowercase();
    if aliases
        .iter()
        .any(|existing| existing.to_lowercase() == key)
    {
        return;
    }
    aliases.push(alias);
}

fn build_font_registration_request(path: &str, family: Option<&str>) -> FontRegistrationRequest {
    let explicit_family = family.and_then(normalize_family_name);
    let raw_stem = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(normalize_family_name);
    let lowercase_stem = raw_stem.as_ref().map(|stem| stem.to_lowercase());

    let family = explicit_family
        .clone()
        .or_else(|| lowercase_stem.clone())
        .or_else(|| raw_stem.clone())
        .unwrap_or_else(|| "custom-font".to_string());

    let mut aliases = Vec::new();
    if let Some(explicit_family) = explicit_family {
        push_alias(&mut aliases, explicit_family);
    }
    if let Some(raw_stem) = raw_stem {
        push_alias(&mut aliases, raw_stem);
    }
    if let Some(lowercase_stem) = lowercase_stem {
        push_alias(&mut aliases, lowercase_stem);
    }
    push_alias(&mut aliases, family.clone());

    FontRegistrationRequest { family, aliases }
}

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
/// Returns the font family key on success, or empty string on failure.
#[op2]
#[string]
pub(crate) fn op_load_font(
    state: &mut OpState,
    #[string] path: String,
    #[string] family: Option<String>,
) -> String {
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

    let request = build_font_registration_request(&path, family.as_deref());

    let bytes = Arc::new(bytes);
    let aliases = Arc::new(request.aliases.clone());

    // Send to render thread for registration.
    let ctx = state.borrow::<CanvasOpState>();
    match send_render_with_resp_sync(ctx, OP_LOAD_FONT, |resp| RenderCommand::LoadFont {
        family: request.family.clone(),
        aliases: aliases.clone(),
        bytes: bytes.clone(),
        resp,
    }) {
        Ok(family) => {
            info!(
                "op_load_font: loaded '{}' as '{}' with aliases {:?}",
                path, family, request.aliases
            );
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
    use super::{build_font_registration_request, resolve_font_src_path};
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

    #[test]
    fn explicit_family_becomes_canonical_registration_key() {
        let request =
            build_font_registration_request("fonts/NotoSans-Regular.ttf", Some("Brand Sans"));
        assert_eq!(request.family, "Brand Sans");
        assert_eq!(
            request.aliases,
            vec![
                "Brand Sans".to_string(),
                "NotoSans-Regular".to_string(),
                "notosans-regular".to_string()
            ]
        );
    }

    #[test]
    fn file_stem_stays_backward_compatible_without_explicit_family() {
        let request = build_font_registration_request("fonts/MyFont.ttf", None);
        assert_eq!(request.family, "myfont");
        assert_eq!(
            request.aliases,
            vec!["MyFont".to_string(), "myfont".to_string()]
        );
    }
}
