//! Synchronous storage (KV) operations.
//!
//! Uses blocking `std::fs` calls, called directly on the V8 thread.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use shared::error::{EngineError, ErrorCode, io_error_to_error_code};

use crate::{
    pools::PoolError,
    scheduler::IoScheduler,
    task::{IoRequest, PriorityClass, RequestKind},
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[inline]
fn io_err(e: std::io::Error) -> EngineError {
    let detail = e.to_string();
    let code = io_error_to_error_code(&e);
    EngineError::new(code).with_detail(detail)
}

#[inline]
fn pool_err(err: PoolError) -> EngineError {
    match err {
        PoolError::Closed => {
            EngineError::new(ErrorCode::IoError).with_detail("IO worker pool closed")
        }
    }
}

#[inline]
fn storage_priority(request: RequestKind) -> PriorityClass {
    match request {
        RequestKind::Sync => PriorityClass::ForegroundBlocking,
        RequestKind::Async => PriorityClass::ForegroundAsync,
    }
}

#[inline]
fn storage_get_request(request: RequestKind, estimated_bytes: usize) -> IoRequest {
    IoRequest::StorageGet {
        request,
        priority: storage_priority(request),
        estimated_bytes,
    }
}

#[inline]
fn storage_mutate_request(request: RequestKind) -> IoRequest {
    IoRequest::StorageMutate {
        request,
        priority: storage_priority(request),
    }
}

#[inline]
fn storage_info_request(request: RequestKind) -> IoRequest {
    IoRequest::StorageInfo {
        request,
        priority: storage_priority(request),
    }
}

#[inline]
fn lock_err<T>(err: std::sync::PoisonError<T>) -> EngineError {
    EngineError::new(ErrorCode::IoError).with_detail(format!("storage lock error: {err}"))
}

/// Decode a hex filename back to the original storage key.
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

// ---------------------------------------------------------------------------
// StorageTotals: optional cache for directory size tracking
// ---------------------------------------------------------------------------

/// Tracks aggregate byte size per storage directory to avoid O(n) re-scans
/// on every `storage_set`.  Populated lazily on first write, then maintained
/// incrementally by set/remove/clear.
pub struct StorageTotals {
    totals: std::collections::HashMap<PathBuf, usize>,
}

impl StorageTotals {
    pub fn new() -> Self {
        Self {
            totals: std::collections::HashMap::new(),
        }
    }

    /// Get cached total or scan and cache it.
    fn get_or_scan(&mut self, dir: &Path) -> Result<usize, EngineError> {
        if let Some(&cached) = self.totals.get(dir) {
            return Ok(cached);
        }
        let mut sum: usize = 0;
        let rd = std::fs::read_dir(dir).map_err(io_err)?;
        for entry_result in rd {
            let entry = entry_result.map_err(io_err)?;
            sum += entry.metadata().map(|m| m.len() as usize).unwrap_or(0);
        }
        self.totals.insert(dir.to_path_buf(), sum);
        Ok(sum)
    }

    /// Update the cached total after a write.
    fn update(&mut self, dir: &Path, old_size: usize, new_size: usize) {
        if let Some(total) = self.totals.get_mut(dir) {
            *total = total.saturating_sub(old_size) + new_size;
        }
    }

    /// Subtract removed bytes from the cache.
    fn subtract(&mut self, dir: &Path, removed_size: usize) {
        if let Some(total) = self.totals.get_mut(dir) {
            *total = total.saturating_sub(removed_size);
        }
    }

    /// Reset a directory total to zero.
    fn reset(&mut self, dir: &Path) {
        self.totals.insert(dir.to_path_buf(), 0);
    }

    /// Whether we have a cached total for this directory.
    fn has_cached(&self, dir: &Path) -> bool {
        self.totals.contains_key(dir)
    }

    /// Clear all cached totals.
    pub fn clear(&mut self) {
        self.totals.clear();
    }
}

// ---------------------------------------------------------------------------
// Storage operations
// ---------------------------------------------------------------------------

/// Read a storage value. Returns empty string if the file does not exist.
pub fn storage_get(path: &str) -> Result<String, EngineError> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(io_err(e)),
    }
}

pub fn storage_get_sync_with_scheduler(
    scheduler: Arc<IoScheduler>,
    path: String,
) -> Result<String, EngineError> {
    let estimated_bytes = std::fs::metadata(&path)
        .map(|meta| meta.len() as usize)
        .unwrap_or(0);
    let request = storage_get_request(RequestKind::Sync, estimated_bytes);
    scheduler
        .run_sync(&request, move || storage_get(&path))
        .map_err(pool_err)?
}

pub async fn storage_get_with_scheduler(
    scheduler: Arc<IoScheduler>,
    path: String,
    request: RequestKind,
) -> Result<String, EngineError> {
    let estimated_bytes = std::fs::metadata(&path)
        .map(|meta| meta.len() as usize)
        .unwrap_or(0);
    let req = storage_get_request(request, estimated_bytes);

    match request {
        RequestKind::Sync => scheduler
            .run_sync(&req, move || storage_get(&path))
            .map_err(pool_err)?,
        RequestKind::Async => scheduler
            .run_async(req, move || storage_get(&path))
            .await
            .map_err(pool_err)?,
    }
}

pub fn storage_set_sync_with_scheduler(
    scheduler: Arc<IoScheduler>,
    dir: String,
    path: String,
    data: String,
    max_total: usize,
    totals: Arc<Mutex<StorageTotals>>,
) -> Result<(), EngineError> {
    let request = storage_mutate_request(RequestKind::Sync);
    scheduler
        .run_sync(&request, move || {
            let mut totals = totals.lock().map_err(lock_err)?;
            storage_set(&dir, &path, &data, max_total, Some(&mut totals))
        })
        .map_err(pool_err)?
}

pub fn storage_remove_sync_with_scheduler(
    scheduler: Arc<IoScheduler>,
    path: String,
    totals: Arc<Mutex<StorageTotals>>,
) -> Result<(), EngineError> {
    let request = storage_mutate_request(RequestKind::Sync);
    scheduler
        .run_sync(&request, move || {
            let mut totals = totals.lock().map_err(lock_err)?;
            storage_remove(&path, Some(&mut totals))
        })
        .map_err(pool_err)?
}

pub fn storage_clear_sync_with_scheduler(
    scheduler: Arc<IoScheduler>,
    dir: String,
    totals: Arc<Mutex<StorageTotals>>,
) -> Result<(), EngineError> {
    let request = storage_mutate_request(RequestKind::Sync);
    scheduler
        .run_sync(&request, move || {
            let mut totals = totals.lock().map_err(lock_err)?;
            storage_clear(&dir, Some(&mut totals))
        })
        .map_err(pool_err)?
}

pub fn storage_info_sync_with_scheduler(
    scheduler: Arc<IoScheduler>,
    dir: String,
    limit_size_kb: u32,
) -> Result<String, EngineError> {
    let request = storage_info_request(RequestKind::Sync);
    scheduler
        .run_sync(&request, move || storage_info(&dir, limit_size_kb))
        .map_err(pool_err)?
}

/// Write a storage value, enforcing a total-size quota for the directory.
///
/// `totals` is an optional mutable reference to a `StorageTotals` cache.
/// When provided, it avoids full directory scans on repeated writes.
pub fn storage_set(
    dir: &str,
    path: &str,
    data: &str,
    max_total: usize,
    mut totals: Option<&mut StorageTotals>,
) -> Result<(), EngineError> {
    std::fs::create_dir_all(dir).map_err(io_err)?;

    // Existing size of the target key (0 if new).
    let existing_size = std::fs::metadata(path)
        .map(|m| m.len() as usize)
        .unwrap_or(0);

    let dir_path = PathBuf::from(dir);

    let total = match totals {
        Some(ref mut t) => t.get_or_scan(&dir_path)?,
        None => {
            // No cache: full directory scan.
            let mut sum: usize = 0;
            let rd = std::fs::read_dir(dir).map_err(io_err)?;
            for entry_result in rd {
                let entry = entry_result.map_err(io_err)?;
                sum += entry.metadata().map(|m| m.len() as usize).unwrap_or(0);
            }
            sum
        }
    };

    if total.saturating_sub(existing_size) + data.len() > max_total {
        return Err(EngineError::new(ErrorCode::IoError)
            .with_detail("setStorage:fail storage limit exceeded"));
    }

    std::fs::write(path, data).map_err(io_err)?;

    // Update cached total.
    if let Some(t) = totals {
        t.update(&dir_path, existing_size, data.len());
    }

    Ok(())
}

/// Remove a storage file. Silent on NotFound.
///
/// If `totals` is provided, updates the cached directory size.
pub fn storage_remove(path: &str, totals: Option<&mut StorageTotals>) -> Result<(), EngineError> {
    let parent_key = Path::new(path).parent().map(|p| p.to_path_buf());

    // Only query file size when the cache is populated for this directory.
    let need_size = match (&parent_key, &totals) {
        (Some(k), Some(t)) => t.has_cached(k),
        _ => false,
    };
    let removed_size = if need_size {
        std::fs::metadata(path)
            .map(|m| m.len() as usize)
            .unwrap_or(0)
    } else {
        0
    };

    match std::fs::remove_file(path) {
        Ok(()) => {
            if let (Some(key), Some(t)) = (parent_key, totals) {
                if removed_size > 0 {
                    t.subtract(&key, removed_size);
                }
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err(e)),
    }
}

/// Remove all files in the storage directory.
///
/// If `totals` is provided, resets the cached total to zero.
pub fn storage_clear(dir: &str, totals: Option<&mut StorageTotals>) -> Result<(), EngineError> {
    match std::fs::read_dir(dir) {
        Ok(rd) => {
            for entry_result in rd {
                let entry = entry_result.map_err(io_err)?;
                let _ = std::fs::remove_file(entry.path());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_err(e)),
    }
    if let Some(t) = totals {
        t.reset(Path::new(dir));
    }
    Ok(())
}

/// Enumerate storage keys and sizes. Returns a JSON string with structure:
/// `{"keys":[...], "currentSize": <kb>, "limitSize": <limit_size_kb>}`
pub fn storage_info(dir: &str, limit_size_kb: u32) -> Result<String, EngineError> {
    let mut keys: Vec<String> = Vec::new();
    let mut total_bytes: u64 = 0;

    match std::fs::read_dir(dir) {
        Ok(rd) => {
            for entry_result in rd {
                let entry = entry_result.map_err(io_err)?;
                if let Some(name) = entry.file_name().to_str() {
                    if let Some(key) = hex_to_key(name) {
                        keys.push(key);
                    }
                }
                total_bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_err(e)),
    }

    let keys_json: String = keys
        .iter()
        .map(|k| format!("\"{}\"", json_escape(k)))
        .collect::<Vec<_>>()
        .join(",");

    let current_size_kb = (total_bytes + 1023) / 1024;

    Ok(format!(
        "{{\"keys\":[{keys_json}],\"currentSize\":{current_size_kb},\"limitSize\":{limit_size_kb}}}"
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::scheduler::IoScheduler;

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("migo_storops_{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Convert a key string to a hex filename (matches the storage encoding).
    fn key_to_hex(key: &str) -> String {
        key.as_bytes()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }

    #[test]
    fn storage_get_existing() {
        let dir = tmp_dir("get_existing");
        let path = dir.join("value.txt");
        std::fs::write(&path, "hello").unwrap();

        let val = storage_get(path.to_str().unwrap()).unwrap();
        assert_eq!(val, "hello");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn storage_get_missing() {
        let val = storage_get("/nonexistent_storage_get_test").unwrap();
        assert_eq!(val, "");
    }

    #[test]
    fn storage_get_sync_stays_inline_for_cheap_path() {
        let dir = tmp_dir("get_scheduler_inline");
        let path = dir.join("value.txt");
        std::fs::write(&path, "hello-inline").unwrap();

        let scheduler = Arc::new(IoScheduler::new(29));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let value = runtime
            .block_on(storage_get_with_scheduler(
                Arc::clone(&scheduler),
                path.to_string_lossy().into_owned(),
                crate::task::RequestKind::Sync,
            ))
            .unwrap();

        assert_eq!(value, "hello-inline");
        assert_eq!(scheduler.pools().spawned_pool_count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn storage_set_within_quota() {
        let dir = tmp_dir("set_quota");
        let path = dir.join("key1");

        storage_set(
            dir.to_str().unwrap(),
            path.to_str().unwrap(),
            "value1",
            1024,
            None,
        )
        .unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "value1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn storage_set_exceeds_quota() {
        let dir = tmp_dir("set_over_quota");
        let path1 = dir.join("key1");
        let path2 = dir.join("key2");

        // Write 8 bytes first.
        std::fs::write(&path1, "12345678").unwrap();

        // Try writing 5 more with quota of 10 (total would be 13 > 10).
        let err = storage_set(
            dir.to_str().unwrap(),
            path2.to_str().unwrap(),
            "12345",
            10,
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("limit exceeded"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn storage_set_with_totals_cache() {
        let dir = tmp_dir("set_cached");
        let path1 = dir.join("key1");
        let path2 = dir.join("key2");
        let mut totals = StorageTotals::new();

        storage_set(
            dir.to_str().unwrap(),
            path1.to_str().unwrap(),
            "aaa",
            1024,
            Some(&mut totals),
        )
        .unwrap();

        storage_set(
            dir.to_str().unwrap(),
            path2.to_str().unwrap(),
            "bbb",
            1024,
            Some(&mut totals),
        )
        .unwrap();

        // Overwrite key1 with bigger value.
        storage_set(
            dir.to_str().unwrap(),
            path1.to_str().unwrap(),
            "aaaa",
            1024,
            Some(&mut totals),
        )
        .unwrap();

        // Check the cache is accurate: key1=4 + key2=3 = 7
        let cached = totals.get_or_scan(&dir).unwrap();
        assert_eq!(cached, 7);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_storage_mutate_and_info_use_scheduler_worker_path() {
        let dir = tmp_dir("sync_scheduler_mutate");
        let path = dir.join("key1");
        let scheduler = Arc::new(IoScheduler::new(41));
        let totals = Arc::new(Mutex::new(StorageTotals::new()));

        storage_set_sync_with_scheduler(
            Arc::clone(&scheduler),
            dir.to_string_lossy().into_owned(),
            path.to_string_lossy().into_owned(),
            "value1".to_string(),
            1024,
            Arc::clone(&totals),
        )
        .unwrap();

        let info = storage_info_sync_with_scheduler(
            Arc::clone(&scheduler),
            dir.to_string_lossy().into_owned(),
            100,
        )
        .unwrap();

        storage_remove_sync_with_scheduler(
            Arc::clone(&scheduler),
            path.to_string_lossy().into_owned(),
            Arc::clone(&totals),
        )
        .unwrap();

        storage_clear_sync_with_scheduler(
            Arc::clone(&scheduler),
            dir.to_string_lossy().into_owned(),
            Arc::clone(&totals),
        )
        .unwrap();

        assert!(info.contains("\"key1\""));
        assert_eq!(scheduler.pools().spawned_pool_count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn storage_remove_existing() {
        let dir = tmp_dir("rm_existing");
        let path = dir.join("key");
        std::fs::write(&path, "x").unwrap();

        storage_remove(path.to_str().unwrap(), None).unwrap();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn storage_remove_missing() {
        // Should not error on missing file.
        storage_remove("/nonexistent_storage_rm_test", None).unwrap();
    }

    #[test]
    fn storage_remove_updates_totals() {
        let dir = tmp_dir("rm_totals");
        let path = dir.join("key1");
        std::fs::write(&path, "12345").unwrap();

        let mut totals = StorageTotals::new();
        // Populate the cache.
        totals.get_or_scan(&dir).unwrap();

        storage_remove(path.to_str().unwrap(), Some(&mut totals)).unwrap();
        assert_eq!(*totals.totals.get(dir.as_path()).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn storage_clear_removes_all() {
        let dir = tmp_dir("clear_all");
        std::fs::write(dir.join("a"), "1").unwrap();
        std::fs::write(dir.join("b"), "2").unwrap();

        let mut totals = StorageTotals::new();
        storage_clear(dir.to_str().unwrap(), Some(&mut totals)).unwrap();

        // Directory should be empty (but still exist).
        assert!(dir.exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        assert_eq!(*totals.totals.get(dir.as_path()).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn storage_clear_missing_dir() {
        // Should not error on missing directory.
        storage_clear("/nonexistent_storage_clear_test", None).unwrap();
    }

    #[test]
    fn storage_info_lists_keys() {
        let dir = tmp_dir("info_keys");
        // Write files named as hex-encoded keys.
        let hex_key = key_to_hex("my_key");
        std::fs::write(dir.join(&hex_key), "value").unwrap();

        let json = storage_info(dir.to_str().unwrap(), 100).unwrap();
        assert!(json.contains("\"my_key\""));
        assert!(json.contains("\"limitSize\":100"));
        assert!(json.contains("\"currentSize\":"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn storage_info_missing_dir() {
        let json = storage_info("/nonexistent_storage_info_dir", 50).unwrap();
        assert!(json.contains("\"keys\":[]"));
        assert!(json.contains("\"currentSize\":0"));
    }

    #[test]
    fn hex_to_key_roundtrip() {
        let original = "hello_world";
        let hex = key_to_hex(original);
        assert_eq!(hex_to_key(&hex), Some(original.to_string()));
    }

    #[test]
    fn hex_to_key_invalid() {
        assert_eq!(hex_to_key("zz"), None); // invalid hex
        assert_eq!(hex_to_key("abc"), None); // odd length
    }

    #[test]
    fn json_escape_special_chars() {
        assert_eq!(json_escape(r#"a"b\c"#), r#"a\"b\\c"#);
        assert_eq!(json_escape("a\nb\rc\td"), "a\\nb\\rc\\td");
    }
}
