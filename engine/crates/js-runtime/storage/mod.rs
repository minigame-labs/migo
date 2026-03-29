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

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use deno_core::{Extension, JsBuffer, OpState, op2};
use deno_error::JsErrorBox;
use shared::error::EngineError;
use shared::op_state::HostOpState;
use shared::protocol::{
    self,
    io_cmd::{IOCmd, IOCmdResp},
};

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
    state
        .borrow::<HostOpState>()
        .app_files_dir
        .join(STORAGE_DIR)
}

#[inline]
fn buffer_url_dir(state: &OpState) -> PathBuf {
    state
        .borrow::<HostOpState>()
        .app_cache_dir
        .join(BUFFER_URL_DIR)
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
pub fn op_storage_get(state: &mut OpState, #[string] key: &str) -> Result<String, JsErrorBox> {
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
        return Err(JsErrorBox::generic("setStorage:fail data exceeds max size"));
    }

    let dir = storage_dir(state);
    ensure_dir(&dir)?;

    let hex_name = key_to_hex(key);
    let target = dir.join(&hex_name);

    // Write to a temp file first, then atomically rename.  This prevents
    // a TOCTOU window between the quota check and the final write.
    let tmp_name = format!(".{}.tmp", &hex_name);
    let tmp_path = dir.join(&tmp_name);
    fs::write(&tmp_path, value)
        .map_err(|e| JsErrorBox::generic(format!("setStorage:fail {e}")))?;

    // Now compute total size (the temp file is included in the listing).
    // Subtract the temp file and any pre-existing target from the total,
    // then add the new value size to get the post-write total.
    let mut total: usize = 0;
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            // Skip our temp file and the old target (we'll replace both).
            if name_str == tmp_name || name_str == hex_name {
                continue;
            }
            total += entry.metadata().map(|m| m.len() as usize).unwrap_or(0);
        }
    }
    total += value_len;

    if total > MAX_TOTAL_SIZE {
        // Quota exceeded — remove the temp file and report error.
        let _ = fs::remove_file(&tmp_path);
        return Err(JsErrorBox::generic(
            "setStorage:fail storage limit exceeded",
        ));
    }

    // Atomic rename: replaces old value if present.
    fs::rename(&tmp_path, &target)
        .map_err(|e| JsErrorBox::generic(format!("setStorage:fail {e}")))?;
    Ok(())
}

#[op2(fast)]
pub fn op_storage_remove(state: &mut OpState, #[string] key: &str) -> Result<(), JsErrorBox> {
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

// ==================== Async Storage Ops ====================

#[derive(Debug, thiserror::Error, deno_error::JsError)]
pub enum StorageError {
    #[class("StorageError")]
    #[error("{0}")]
    Message(String),
}

impl From<EngineError> for StorageError {
    #[inline]
    fn from(e: EngineError) -> Self {
        match &e.detail {
            Some(d) => StorageError::Message(format!("{} ({})", e.msg, d)),
            None => StorageError::Message(e.msg.to_string()),
        }
    }
}

#[op2(async(lazy), fast)]
#[string]
pub async fn op_storage_get_async(
    state: Rc<RefCell<OpState>>,
    #[string] key: String,
) -> Result<String, StorageError> {
    let (path, tx) = {
        let st = state.borrow();
        let hos = st.borrow::<HostOpState>();
        let path = hos.app_files_dir.join(STORAGE_DIR).join(key_to_hex(&key));
        (path.to_string_lossy().into_owned(), hos.io_tx.clone())
    };
    protocol::send_fs_with_resp_async(&tx, |resp_tx| IOCmd::StorageGet {
        path,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(StorageError::from)
}

#[op2(async(lazy), fast)]
pub async fn op_storage_set_async(
    state: Rc<RefCell<OpState>>,
    #[string] key: String,
    #[string] value: String,
) -> Result<(), StorageError> {
    if value.len() > MAX_VALUE_SIZE {
        return Err(StorageError::Message(
            "setStorage:fail data exceeds max size".into(),
        ));
    }

    let (dir, path, tx) = {
        let st = state.borrow();
        let hos = st.borrow::<HostOpState>();
        let dir = hos.app_files_dir.join(STORAGE_DIR);
        let path = dir.join(key_to_hex(&key));
        (
            dir.to_string_lossy().into_owned(),
            path.to_string_lossy().into_owned(),
            hos.io_tx.clone(),
        )
    };
    protocol::send_fs_with_resp_async(&tx, |resp_tx| IOCmd::StorageSet {
        dir,
        path,
        data: value,
        max_total: MAX_TOTAL_SIZE,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(StorageError::from)
}

#[op2(async(lazy), fast)]
pub async fn op_storage_remove_async(
    state: Rc<RefCell<OpState>>,
    #[string] key: String,
) -> Result<(), StorageError> {
    let (path, tx) = {
        let st = state.borrow();
        let hos = st.borrow::<HostOpState>();
        let path = hos.app_files_dir.join(STORAGE_DIR).join(key_to_hex(&key));
        (path.to_string_lossy().into_owned(), hos.io_tx.clone())
    };
    protocol::send_fs_with_resp_async(&tx, |resp_tx| IOCmd::StorageRemove {
        path,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(StorageError::from)
}

#[op2(async(lazy), fast)]
pub async fn op_storage_clear_async(state: Rc<RefCell<OpState>>) -> Result<(), StorageError> {
    let (dir, tx) = {
        let st = state.borrow();
        let hos = st.borrow::<HostOpState>();
        let dir = hos.app_files_dir.join(STORAGE_DIR);
        (dir.to_string_lossy().into_owned(), hos.io_tx.clone())
    };
    protocol::send_fs_with_resp_async(&tx, |resp_tx| IOCmd::StorageClear {
        dir,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(StorageError::from)
}

#[op2(async(lazy), fast)]
#[string]
pub async fn op_storage_info_async(state: Rc<RefCell<OpState>>) -> Result<String, StorageError> {
    let (dir, tx) = {
        let st = state.borrow();
        let hos = st.borrow::<HostOpState>();
        let dir = hos.app_files_dir.join(STORAGE_DIR);
        (dir.to_string_lossy().into_owned(), hos.io_tx.clone())
    };
    protocol::send_fs_with_resp_async(&tx, |resp_tx| IOCmd::StorageInfo {
        dir,
        limit_size_kb: LIMIT_SIZE_KB,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map_err(StorageError::from)
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
pub fn op_revoke_buffer_url(state: &mut OpState, #[string] url: &str) -> Result<(), JsErrorBox> {
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
        op_storage_get_async,
        op_storage_set_async,
        op_storage_remove_async,
        op_storage_clear_async,
        op_storage_info_async,
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

pub fn storage_lazy_extensions() -> Vec<Extension> {
    vec![host_v8_storage::lazy_init()]
}
