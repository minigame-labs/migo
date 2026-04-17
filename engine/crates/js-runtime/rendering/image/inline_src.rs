//! Inline image-source loaders: `data:` URLs and `http(s)://` URLs.
//!
//! These bypass the filesystem VFS and feed decoded RGBA directly into
//! the GPU upload pipeline.  Kept in a dedicated module so the mount
//! table / VFS paths in [`super::resolve_local_src`] stay small and
//! focused on local resources.

use std::rc::Rc;

use base64::Engine as _;
use deno_core::OpState;
use deno_error::JsErrorBox;
use shared::error::{EngineError, EngineResult, ErrorCode};
use shared::protocol::io_cmd::NormalizedImage;
use tracing::{debug, warn};

/// Parsed `data:` URL payload.
pub struct DataUrlPayload {
    pub bytes: Vec<u8>,
    /// Lowercase MIME type, e.g. `"image/png"`; used only as a decoder
    /// hint.  Empty string if the URL omitted the media type.
    pub mime: String,
}

/// Parse an RFC 2397 `data:` URL.
///
/// Supports both base64 and percent-encoded payloads; tolerates the
/// permissive MIME / charset variations that show up in real small-game
/// bundles (`data:image/png;base64,…`, `data:;base64,…`, `data:,raw%20text`).
pub fn parse_data_url(src: &str) -> EngineResult<DataUrlPayload> {
    let rest = src.strip_prefix("data:").ok_or_else(|| {
        EngineError::new(ErrorCode::ImageReadError)
            .with_msg("invalid data URL")
            .with_detail("missing data: prefix")
    })?;
    // `mediatype,payload` — comma separator MUST exist.
    let (meta, payload) = rest.split_once(',').ok_or_else(|| {
        EngineError::new(ErrorCode::ImageReadError)
            .with_msg("malformed data URL")
            .with_detail("no comma separator between meta and payload")
    })?;

    let mut is_base64 = false;
    let mut mime = String::new();
    for (i, param) in meta.split(';').enumerate() {
        let p = param.trim();
        if p.eq_ignore_ascii_case("base64") {
            is_base64 = true;
        } else if i == 0 && !p.is_empty() {
            mime = p.to_ascii_lowercase();
        }
        // Silently ignore `charset=utf-8` etc — we treat the payload as
        // opaque bytes for the image decoder.
    }

    let bytes = if is_base64 {
        base64::engine::general_purpose::STANDARD
            .decode(payload.trim())
            .map_err(|e| {
                EngineError::new(ErrorCode::ImageReadError)
                    .with_msg("data URL base64 decode failed")
                    .with_detail(e.to_string())
            })?
    } else {
        percent_encoding::percent_decode_str(payload)
            .collect::<Vec<u8>>()
    };

    Ok(DataUrlPayload { bytes, mime })
}

/// Fetch an HTTP/HTTPS image and return its body bytes.
///
/// Subject to the host's `NetworkPolicy` (enforced by the shared
/// reqwest client pool) — so `http://` requests and off-whitelist
/// hosts are rejected consistently with `fetch()` calls from JS.
pub async fn fetch_http_image(
    state: Rc<std::cell::RefCell<OpState>>,
    url: &str,
) -> EngineResult<Vec<u8>> {
    let client = {
        let mut st = state.borrow_mut();
        crate::network::fetch::get_or_create_client_from_state(&mut st, false).map_err(|e| {
            EngineError::new(ErrorCode::IoError)
                .with_msg("http client not available")
                .with_detail(e.to_string())
        })?
    };
    let resp = client.get(url).send().await.map_err(|e| {
        EngineError::new(ErrorCode::IoError)
            .with_msg("image fetch failed")
            .with_detail(e.to_string())
    })?;
    if !resp.status().is_success() {
        return Err(EngineError::new(ErrorCode::IoError)
            .with_msg("image fetch returned non-2xx")
            .with_detail(format!("url={}, status={}", url, resp.status())));
    }
    resp.bytes().await.map(|b| b.to_vec()).map_err(|e| {
        EngineError::new(ErrorCode::IoError)
            .with_msg("image body read failed")
            .with_detail(e.to_string())
    })
}

/// Decode raw image bytes into a normalised RGBA8 buffer, applying an
/// optional target-size resize.
pub fn decode_inline_bytes(
    bytes: &[u8],
    hint_mime: Option<&str>,
    target_width: Option<u32>,
    target_height: Option<u32>,
) -> EngineResult<NormalizedImage> {
    if bytes.is_empty() {
        return Err(EngineError::new(ErrorCode::ImageReadError)
            .with_msg("empty image payload"));
    }
    let decoded = io::decode_image_fast(bytes, hint_mime).map_err(|e| {
        warn!("decode_inline_bytes failed ({} bytes): {:?}", bytes.len(), e);
        e
    })?;

    match (target_width, target_height) {
        (Some(tw), Some(th)) if tw > 0 && th > 0 => {
            debug!(
                "decode_inline_bytes resize {}x{} -> {}x{}",
                decoded.width, decoded.height, tw, th
            );
            Ok(io::resize_image(decoded, tw, th))
        }
        _ => Ok(decoded),
    }
}

/// Quick `Err` builder for unsupported src prefixes routed to this
/// module incorrectly — kept behind a helper so the op path stays
/// readable.
#[inline]
pub fn unsupported_scheme_err(src: &str) -> JsErrorBox {
    JsErrorBox::generic(format!(
        "unsupported image src scheme: {}",
        src.split_once(':').map(|(s, _)| s).unwrap_or(src)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_base64_png_data_url() {
        // 1x1 red PNG, base64-encoded.
        let src = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let payload = parse_data_url(src).unwrap();
        assert_eq!(payload.mime, "image/png");
        assert!(payload.bytes.starts_with(b"\x89PNG"));
    }

    #[test]
    fn parse_percent_encoded_data_url() {
        // `data:,hello%20world` — raw bytes after percent decoding.
        let payload = parse_data_url("data:,hello%20world").unwrap();
        assert_eq!(payload.mime, "");
        assert_eq!(payload.bytes, b"hello world");
    }

    #[test]
    fn parse_data_url_without_meta() {
        // `data:;base64,SGVsbG8=` — no MIME type but `;base64` token.
        let payload = parse_data_url("data:;base64,SGVsbG8=").unwrap();
        assert_eq!(payload.mime, "");
        assert_eq!(payload.bytes, b"Hello");
    }

    #[test]
    fn parse_data_url_missing_comma_errors() {
        assert!(parse_data_url("data:image/png;base64_noseparator").is_err());
    }

    #[test]
    fn parse_data_url_bad_base64_errors() {
        // `@#$` is not valid base64; should surface a decode error.
        assert!(parse_data_url("data:image/png;base64,@#$%").is_err());
    }
}
