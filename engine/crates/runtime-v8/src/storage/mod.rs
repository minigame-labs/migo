//! Key-value storage ops and buffer URL management.
//!
//! Backed by an embedded WAL-mode SQLite database opened lazily per
//! session at `{app_files_dir}/kv_storage/storage.db`.  The on-disk
//! layout (a single `storage.db`) is a full replacement of the
//! previous `file-per-key` hex-named layout — see
//! [`migo_io::storage_ops`] and [`migo_io::kv_store`] for the rationale and
//! schema.
//!
//! ## Limits
//!
//! - Single value: 1 MB
//! - Total storage: 10 MB
//!
//! The quota is enforced inside the SQLite transaction via
//! `SELECT SUM(size)`, which is O(n) but n is bounded to a few
//! thousand small-game keys in practice; the extra read is
//! dominated by the single write fsync cost anyway.

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use deno_core::{Extension, JsBuffer, OpState, op2};
use deno_error::JsErrorBox;
use migo_io::storage_ops::{self, StorageInfo};
use migo_io::task::{IoRequest, PriorityClass, RequestKind};
use shared::error::EngineError;
use shared::op_state::HostOpState;

use crate::io_state::IoSchedulerState;

/// Storage directory name under `app_files_dir`.
///
/// The SQLite file itself lives at `{STORAGE_DIR}/storage.db`; the
/// directory layer is kept so per-game cleanup tools that `rm -rf`
/// the folder keep working.
const STORAGE_DIR: &str = "kv_storage";

/// Buffer URL directory name under `app_cache_dir`.
const BUFFER_URL_DIR: &str = "buffer_urls";

/// Maximum size of a single stored value (1 MB).
const MAX_VALUE_SIZE: usize = 1024 * 1024;

/// Maximum total storage size in KB (10 MB = 10240 KB).
const LIMIT_SIZE_KB: u32 = 10240;

/// Maximum total storage size in bytes.
const MAX_TOTAL_BYTES: u64 = LIMIT_SIZE_KB as u64 * 1024;

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

#[inline]
fn get_scheduler(state: &OpState) -> Arc<migo_io::scheduler::IoScheduler> {
    state.borrow::<IoSchedulerState>().0.clone()
}

#[inline]
fn pool_err(err: migo_io::pools::PoolError) -> StorageError {
    StorageError::Message(err.to_string())
}

fn ensure_dir(dir: &std::path::Path) -> Result<(), JsErrorBox> {
    if !dir.exists() {
        fs::create_dir_all(dir)
            .map_err(|e| JsErrorBox::generic(format!("storage: mkdir fail {e}")))?;
    }
    Ok(())
}

/// Convert an engine-layer error to the JS-visible message. Keeps
/// the user-facing string equivalent to the old ops so existing
/// error-match code in games keeps working.
fn js_err(e: EngineError) -> JsErrorBox {
    match &e.detail {
        Some(d) => JsErrorBox::generic(d.clone()),
        None => JsErrorBox::generic(e.msg.to_string()),
    }
}

/// Serialize [`StorageInfo`] into the JSON shape the JS wrapper
/// expects: `{ keys: [...], currentSize: <KB>, limitSize: <KB> }`.
/// Sizes are reported in KiB (ceil), mirroring the legacy format.
fn info_to_json(info: &StorageInfo) -> String {
    // Ceil to KiB so a 1-byte value reports currentSize=1.
    let current_kib = (info.current_bytes + 1023) / 1024;
    let limit_kib = (info.limit_bytes + 1023) / 1024;
    let mut keys = String::with_capacity(info.keys.len() * 16);
    for (i, k) in info.keys.iter().enumerate() {
        if i > 0 {
            keys.push(',');
        }
        keys.push('"');
        // Reuse the serde_json encoder via manual escape — we avoid
        // pulling in serde_json for this tiny use, but still handle
        // the JSON metacharacters that can appear in user keys.
        for c in k.chars() {
            match c {
                '"' => keys.push_str("\\\""),
                '\\' => keys.push_str("\\\\"),
                '\n' => keys.push_str("\\n"),
                '\r' => keys.push_str("\\r"),
                '\t' => keys.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    keys.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => keys.push(c),
            }
        }
        keys.push('"');
    }
    format!(r#"{{"keys":[{keys}],"currentSize":{current_kib},"limitSize":{limit_kib}}}"#)
}

// ==================== Sync Storage Ops ====================

#[op2]
#[string]
pub fn op_storage_get(state: &mut OpState, #[string] key: &str) -> Result<String, JsErrorBox> {
    let scheduler = get_scheduler(state);
    let dir = storage_dir(state);
    // Missing key maps to "" so the JS-side `deserialize("")` contract
    // (return empty string) keeps working without a wire-format change.
    storage_ops::storage_get_sync_with_scheduler(scheduler, dir, key.to_string(), MAX_TOTAL_BYTES)
        .map(|opt| opt.unwrap_or_default())
        .map_err(js_err)
}

#[op2(fast)]
pub fn op_storage_set(
    state: &mut OpState,
    #[string] key: &str,
    #[string] value: &str,
) -> Result<(), JsErrorBox> {
    if value.len() > MAX_VALUE_SIZE {
        return Err(JsErrorBox::generic("setStorage:fail data exceeds max size"));
    }
    let scheduler = get_scheduler(state);
    let dir = storage_dir(state);
    storage_ops::storage_set_sync_with_scheduler(
        scheduler,
        dir,
        key.to_string(),
        value.to_string(),
        MAX_TOTAL_BYTES,
    )
    .map_err(js_err)
}

#[op2(fast)]
pub fn op_storage_remove(state: &mut OpState, #[string] key: &str) -> Result<(), JsErrorBox> {
    let scheduler = get_scheduler(state);
    let dir = storage_dir(state);
    storage_ops::storage_remove_sync_with_scheduler(
        scheduler,
        dir,
        key.to_string(),
        MAX_TOTAL_BYTES,
    )
    .map_err(js_err)
}

#[op2(fast)]
pub fn op_storage_clear(state: &mut OpState) -> Result<(), JsErrorBox> {
    let scheduler = get_scheduler(state);
    let dir = storage_dir(state);
    storage_ops::storage_clear_sync_with_scheduler(scheduler, dir, MAX_TOTAL_BYTES).map_err(js_err)
}

#[op2]
#[string]
pub fn op_storage_info(state: &mut OpState) -> Result<String, JsErrorBox> {
    let scheduler = get_scheduler(state);
    let dir = storage_dir(state);
    let info = storage_ops::storage_info_sync_with_scheduler(scheduler, dir, MAX_TOTAL_BYTES)
        .map_err(js_err)?;
    Ok(info_to_json(&info))
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

/// Route a blocking KvStore call through the IoScheduler as an async
/// task. All four mutate-style async ops share this shape, so folding
/// the boilerplate into one helper keeps the call sites obvious.
async fn run_mutate_async<F>(state: Rc<RefCell<OpState>>, f: F) -> Result<(), StorageError>
where
    F: FnOnce(&std::path::Path) -> Result<(), EngineError> + Send + 'static,
{
    let (scheduler, dir) = {
        let st = state.borrow();
        (get_scheduler(&st), storage_dir(&st))
    };
    scheduler
        .run_async(
            IoRequest::StorageMutate {
                request: RequestKind::Async,
                priority: PriorityClass::from(RequestKind::Async),
            },
            move || f(&dir).map_err(StorageError::from),
        )
        .await
        .map_err(pool_err)?
}

#[op2(async(lazy), fast)]
#[string]
pub async fn op_storage_get_async(
    state: Rc<RefCell<OpState>>,
    #[string] key: String,
) -> Result<String, StorageError> {
    let (scheduler, dir) = {
        let st = state.borrow();
        (get_scheduler(&st), storage_dir(&st))
    };
    storage_ops::storage_get_with_scheduler(
        scheduler,
        dir,
        key,
        MAX_TOTAL_BYTES,
        RequestKind::Async,
    )
    .await
    .map(|opt| opt.unwrap_or_default())
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
    run_mutate_async(state, move |dir| {
        storage_ops::storage_set(dir, &key, &value, MAX_TOTAL_BYTES)
    })
    .await
}

#[op2(async(lazy), fast)]
pub async fn op_storage_remove_async(
    state: Rc<RefCell<OpState>>,
    #[string] key: String,
) -> Result<(), StorageError> {
    run_mutate_async(state, move |dir| {
        storage_ops::storage_remove(dir, &key, MAX_TOTAL_BYTES)
    })
    .await
}

#[op2(async(lazy), fast)]
pub async fn op_storage_clear_async(state: Rc<RefCell<OpState>>) -> Result<(), StorageError> {
    run_mutate_async(state, move |dir| {
        storage_ops::storage_clear(dir, MAX_TOTAL_BYTES)
    })
    .await
}

#[op2(async(lazy), fast)]
#[string]
pub async fn op_storage_info_async(state: Rc<RefCell<OpState>>) -> Result<String, StorageError> {
    let (scheduler, dir) = {
        let st = state.borrow();
        (get_scheduler(&st), storage_dir(&st))
    };
    let info = scheduler
        .run_async(
            IoRequest::StorageInfo {
                request: RequestKind::Async,
                priority: PriorityClass::from(RequestKind::Async),
            },
            move || storage_ops::storage_info(&dir, MAX_TOTAL_BYTES).map_err(StorageError::from),
        )
        .await
        .map_err(pool_err)??;
    Ok(info_to_json(&info))
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
        dir "src/storage",
        "01_storage.js",
    ],
);

pub fn storage_extensions() -> Vec<Extension> {
    vec![host_v8_storage::init()]
}

pub fn storage_lazy_extensions() -> Vec<Extension> {
    vec![host_v8_storage::lazy_init()]
}
