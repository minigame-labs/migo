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
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use deno_core::{Extension, JsBuffer, OpState, op2};
use deno_error::JsErrorBox;
use io::storage_ops;
use io::task::{IoRequest, PriorityClass, RequestKind};
use shared::error::EngineError;
use shared::op_state::HostOpState;

use crate::io_state::IoSchedulerState;

/// Shared per-session storage totals cache, stored in OpState.
/// Avoids O(n) directory scans on every `setStorage` after the first write.
#[derive(Clone)]
struct StorageTotalsState(Arc<Mutex<storage_ops::StorageTotals>>);

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

#[inline]
fn get_scheduler(state: &OpState) -> Arc<io::scheduler::IoScheduler> {
    state.borrow::<IoSchedulerState>().0.clone()
}

#[inline]
fn pool_err(err: io::pools::PoolError) -> StorageError {
    StorageError::Message(err.to_string())
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
    storage_ops::storage_get_sync_with_scheduler(
        get_scheduler(state),
        path.to_string_lossy().into_owned(),
    )
    .map_err(|e| JsErrorBox::generic(format!("getStorage:fail {e}")))
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
    let totals_arc = state.borrow::<StorageTotalsState>().0.clone();
    let scheduler = get_scheduler(state);

    let dir_str = dir.to_string_lossy().into_owned();
    let path = dir.join(key_to_hex(key));
    let path_str = path.to_string_lossy().into_owned();

    storage_ops::storage_set_sync_with_scheduler(
        scheduler,
        dir_str,
        path_str,
        value.to_string(),
        MAX_TOTAL_SIZE,
        totals_arc,
    )
    .map_err(|e| match &e.detail {
        Some(d) => JsErrorBox::generic(d.clone()),
        None => JsErrorBox::generic(e.msg.to_string()),
    })
}

#[op2(fast)]
pub fn op_storage_remove(state: &mut OpState, #[string] key: &str) -> Result<(), JsErrorBox> {
    let path = storage_dir(state).join(key_to_hex(key));
    let totals_arc = state.borrow::<StorageTotalsState>().0.clone();
    let scheduler = get_scheduler(state);
    let path_str = path.to_string_lossy().into_owned();

    storage_ops::storage_remove_sync_with_scheduler(scheduler, path_str, totals_arc).map_err(|e| {
        match &e.detail {
            Some(d) => JsErrorBox::generic(d.clone()),
            None => JsErrorBox::generic(e.msg.to_string()),
        }
    })
}

#[op2(fast)]
pub fn op_storage_clear(state: &mut OpState) -> Result<(), JsErrorBox> {
    let dir = storage_dir(state);
    let totals_arc = state.borrow::<StorageTotalsState>().0.clone();
    let scheduler = get_scheduler(state);
    let dir_str = dir.to_string_lossy().into_owned();

    storage_ops::storage_clear_sync_with_scheduler(scheduler, dir_str, totals_arc).map_err(|e| {
        match &e.detail {
            Some(d) => JsErrorBox::generic(d.clone()),
            None => JsErrorBox::generic(e.msg.to_string()),
        }
    })
}

#[op2]
#[string]
pub fn op_storage_info(state: &mut OpState) -> Result<String, JsErrorBox> {
    let dir = storage_dir(state);
    storage_ops::storage_info_sync_with_scheduler(
        get_scheduler(state),
        dir.to_string_lossy().into_owned(),
        LIMIT_SIZE_KB,
    )
    .map_err(|e| match &e.detail {
        Some(d) => JsErrorBox::generic(d.clone()),
        None => JsErrorBox::generic(e.msg.to_string()),
    })
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
    let (scheduler, path) = {
        let st = state.borrow();
        let hos = st.borrow::<HostOpState>();
        (
            get_scheduler(&st),
            hos.app_files_dir
                .join(STORAGE_DIR)
                .join(key_to_hex(&key))
                .to_string_lossy()
                .into_owned(),
        )
    };
    storage_ops::storage_get_with_scheduler(scheduler, path, RequestKind::Async)
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

    let (scheduler, dir, path, totals_arc) = {
        let st = state.borrow();
        let hos = st.borrow::<HostOpState>();
        let dir = hos.app_files_dir.join(STORAGE_DIR);
        let path = dir.join(key_to_hex(&key));
        let totals_arc = st.borrow::<StorageTotalsState>().0.clone();
        (
            get_scheduler(&st),
            dir.to_string_lossy().into_owned(),
            path.to_string_lossy().into_owned(),
            totals_arc,
        )
    };
    let max_total = MAX_TOTAL_SIZE;
    scheduler
        .run_async(
            IoRequest::StorageMutate {
                request: RequestKind::Async,
                priority: PriorityClass::from(RequestKind::Async),
            },
            move || {
                let mut totals = totals_arc
                    .lock()
                    .map_err(|e| StorageError::Message(format!("lock error: {e}")))?;
                storage_ops::storage_set(&dir, &path, &value, max_total, Some(&mut totals))
                    .map_err(StorageError::from)
            },
        )
        .await
        .map_err(pool_err)?
}

#[op2(async(lazy), fast)]
pub async fn op_storage_remove_async(
    state: Rc<RefCell<OpState>>,
    #[string] key: String,
) -> Result<(), StorageError> {
    let (scheduler, path, totals_arc) = {
        let st = state.borrow();
        let hos = st.borrow::<HostOpState>();
        let path = hos
            .app_files_dir
            .join(STORAGE_DIR)
            .join(key_to_hex(&key))
            .to_string_lossy()
            .into_owned();
        let totals_arc = st.borrow::<StorageTotalsState>().0.clone();
        (get_scheduler(&st), path, totals_arc)
    };
    scheduler
        .run_async(
            IoRequest::StorageMutate {
                request: RequestKind::Async,
                priority: PriorityClass::from(RequestKind::Async),
            },
            move || {
                let mut totals = totals_arc
                    .lock()
                    .map_err(|e| StorageError::Message(format!("lock error: {e}")))?;
                storage_ops::storage_remove(&path, Some(&mut totals)).map_err(StorageError::from)
            },
        )
        .await
        .map_err(pool_err)?
}

#[op2(async(lazy), fast)]
pub async fn op_storage_clear_async(state: Rc<RefCell<OpState>>) -> Result<(), StorageError> {
    let (scheduler, dir, totals_arc) = {
        let st = state.borrow();
        let hos = st.borrow::<HostOpState>();
        let dir = hos
            .app_files_dir
            .join(STORAGE_DIR)
            .to_string_lossy()
            .into_owned();
        let totals_arc = st.borrow::<StorageTotalsState>().0.clone();
        (get_scheduler(&st), dir, totals_arc)
    };
    scheduler
        .run_async(
            IoRequest::StorageMutate {
                request: RequestKind::Async,
                priority: PriorityClass::from(RequestKind::Async),
            },
            move || {
                let mut totals = totals_arc
                    .lock()
                    .map_err(|e| StorageError::Message(format!("lock error: {e}")))?;
                storage_ops::storage_clear(&dir, Some(&mut totals)).map_err(StorageError::from)
            },
        )
        .await
        .map_err(pool_err)?
}

#[op2(async(lazy), fast)]
#[string]
pub async fn op_storage_info_async(state: Rc<RefCell<OpState>>) -> Result<String, StorageError> {
    let (scheduler, dir) = {
        let st = state.borrow();
        let hos = st.borrow::<HostOpState>();
        (
            get_scheduler(&st),
            hos.app_files_dir
                .join(STORAGE_DIR)
                .to_string_lossy()
                .into_owned(),
        )
    };
    let limit = LIMIT_SIZE_KB;
    scheduler
        .run_async(
            IoRequest::StorageInfo {
                request: RequestKind::Async,
                priority: PriorityClass::from(RequestKind::Async),
            },
            move || storage_ops::storage_info(&dir, limit).map_err(StorageError::from),
        )
        .await
        .map_err(pool_err)?
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
    state = |state| {
        state.put::<StorageTotalsState>(StorageTotalsState(
            Arc::new(Mutex::new(storage_ops::StorageTotals::new())),
        ));
    },
);

pub fn storage_extensions() -> Vec<Extension> {
    vec![host_v8_storage::init()]
}

pub fn storage_lazy_extensions() -> Vec<Extension> {
    vec![host_v8_storage::lazy_init()]
}
