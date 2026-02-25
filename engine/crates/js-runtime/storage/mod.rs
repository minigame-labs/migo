//! Key-value storage ops and buffer URL management.
//!
//! Provides persistent key-value storage backed by the local file system,
//! with per-key files stored under `{app_files_dir}/kv_storage/`.
//!
//! ## Limits
//!
//! - Single key: 1 MB
//! - Total storage: 10 MB
//!
//! ## File Layout
//!
//! ```text
//! {app_files_dir}/kv_storage/
//!     {hex_encoded_key}.dat   <- type-tagged JSON value
//! {app_cache_dir}/buffer_urls/
//!     {timestamp_hex}         <- raw binary blob
//! ```

use std::fs;
use std::io;
use std::path::PathBuf;

use deno_core::{Extension, JsBuffer, OpState, op2};
use deno_error::JsErrorBox;
use shared::op_state::HostOpState;

/// Storage directory name under `app_files_dir`.
const STORAGE_DIR: &str = "kv_storage";

/// Buffer URL directory name under `app_cache_dir`.
const BUFFER_URL_DIR: &str = "buffer_urls";

/// Maximum size of a single stored value (1 MB).
const MAX_VALUE_SIZE: usize = 1024 * 1024;

/// Maximum total storage size in KB (10 MB = 10240 KB).
const LIMIT_SIZE_KB: u32 = 10240;

/// Maximum total storage size in bytes.
const MAX_TOTAL_SIZE: usize = LIMIT_SIZE_KB as usize * 1024;

// ==================== Path Helpers ====================

#[inline]
fn storage_dir(state: &OpState) -> PathBuf {
    state.borrow::<HostOpState>().app_files_dir.join(STORAGE_DIR)
}

#[inline]
fn buffer_url_dir(state: &OpState) -> PathBuf {
    state.borrow::<HostOpState>().app_cache_dir.join(BUFFER_URL_DIR)
}

/// Encode a storage key to a hex filename.
///
/// This avoids issues with special characters, path separators, and
/// filesystem-reserved names across platforms.
fn key_to_hex(key: &str) -> String {
    let bytes = key.as_bytes();
    let mut hex = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        // Manual hex encoding — avoids `format!` overhead per byte.
        const HEX: &[u8; 16] = b"0123456789abcdef";
        hex.push(HEX[(b >> 4) as usize] as char);
        hex.push(HEX[(b & 0x0f) as usize] as char);
    }
    hex
}

/// Decode a hex filename back to the original key.
fn hex_to_key(hex: &str) -> Option<String> {
    let hex = hex.as_bytes();
    if hex.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for pair in hex.chunks_exact(2) {
        let hi = hex_digit(pair[0])?;
        let lo = hex_digit(pair[1])?;
        bytes.push((hi << 4) | lo);
    }
    String::from_utf8(bytes).ok()
}

#[inline]
fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Escape a string for safe embedding in a JSON string literal.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

fn ensure_dir(dir: &std::path::Path) -> Result<(), JsErrorBox> {
    if !dir.exists() {
        fs::create_dir_all(dir)
            .map_err(|e| JsErrorBox::generic(format!("storage: mkdir fail {e}")))?;
    }
    Ok(())
}

// ==================== Storage Ops ====================

#[op2]
#[string]
pub fn op_storage_get(
    state: &mut OpState,
    #[string] key: &str,
) -> Result<String, JsErrorBox> {
    let path = storage_dir(state).join(key_to_hex(key));
    match fs::read_to_string(&path) {
        Ok(content) => Ok(content),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(JsErrorBox::generic(format!("getStorage:fail {e}"))),
    }
}

#[op2(fast)]
pub fn op_storage_set(
    state: &mut OpState,
    #[string] key: &str,
    #[string] value: &str,
) -> Result<(), JsErrorBox> {
    let value_len = value.len();
    if value_len > MAX_VALUE_SIZE {
        return Err(JsErrorBox::generic(
            "setStorage:fail data exceeds max size",
        ));
    }

    let dir = storage_dir(state);
    ensure_dir(&dir)?;

    let hex_name = key_to_hex(key);
    let target = dir.join(&hex_name);

    // Compute current total and the size of any existing value for this key.
    let existing_size = fs::metadata(&target)
        .map(|m| m.len() as usize)
        .unwrap_or(0);

    let mut total: usize = 0;
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            total += entry.metadata().map(|m| m.len() as usize).unwrap_or(0);
        }
    }

    if total - existing_size + value_len > MAX_TOTAL_SIZE {
        return Err(JsErrorBox::generic(
            "setStorage:fail storage limit exceeded",
        ));
    }

    fs::write(&target, value)
        .map_err(|e| JsErrorBox::generic(format!("setStorage:fail {e}")))?;
    Ok(())
}

#[op2(fast)]
pub fn op_storage_remove(
    state: &mut OpState,
    #[string] key: &str,
) -> Result<(), JsErrorBox> {
    let path = storage_dir(state).join(key_to_hex(key));
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(JsErrorBox::generic(format!("removeStorage:fail {e}"))),
    }
}

#[op2(fast)]
pub fn op_storage_clear(state: &mut OpState) -> Result<(), JsErrorBox> {
    let dir = storage_dir(state);
    if dir.exists() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    Ok(())
}

#[op2]
#[string]
pub fn op_storage_info(state: &mut OpState) -> Result<String, JsErrorBox> {
    let dir = storage_dir(state);
    let mut keys: Vec<String> = Vec::new();
    let mut total_bytes: u64 = 0;

    if dir.exists() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if let Some(key) = hex_to_key(name) {
                        keys.push(key);
                    }
                }
                total_bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }

    // Build JSON without serde.
    let keys_json: String = keys
        .iter()
        .map(|k| format!("\"{}\"", json_escape(k)))
        .collect::<Vec<_>>()
        .join(",");

    let current_size_kb = (total_bytes + 1023) / 1024;

    Ok(format!(
        "{{\"keys\":[{keys_json}],\"currentSize\":{current_size_kb},\"limitSize\":{LIMIT_SIZE_KB}}}"
    ))
}

// ==================== Buffer URL Ops ====================

#[op2]
#[string]
pub fn op_create_buffer_url(
    state: &mut OpState,
    #[buffer] buffer: JsBuffer,
) -> Result<String, JsErrorBox> {
    let dir = buffer_url_dir(state);
    ensure_dir(&dir)?;

    // Unique file name: nanosecond timestamp in hex.
    let id = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("{nanos:x}")
    };

    let path = dir.join(&id);
    fs::write(&path, &*buffer)
        .map_err(|e| JsErrorBox::generic(format!("createBufferURL:fail {e}")))?;

    Ok(path.to_string_lossy().into_owned())
}

#[op2(fast)]
pub fn op_revoke_buffer_url(
    state: &mut OpState,
    #[string] url: &str,
) -> Result<(), JsErrorBox> {
    let dir = buffer_url_dir(state);
    let path = std::path::Path::new(url);
    // Only allow deleting files within the buffer URL directory.
    if path.starts_with(&dir) && path.is_file() {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

// ==================== Extension ====================

deno_core::extension!(
    host_v8_storage,
    deps = [host_v8_base],
    ops = [
        op_storage_get,
        op_storage_set,
        op_storage_remove,
        op_storage_clear,
        op_storage_info,
        op_create_buffer_url,
        op_revoke_buffer_url,
    ],
    esm = [
        dir "storage",
        "01_storage.js",
    ],
);

pub fn storage_extensions() -> Vec<Extension> {
    vec![host_v8_storage::init()]
}
