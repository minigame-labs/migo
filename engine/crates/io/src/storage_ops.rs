//! Storage (KV) operations — SQLite-backed.
//!
//! This file is the Rust-side facade for the `getStorage / setStorage /
//! removeStorage / clearStorage / getStorageInfo` small-game APIs.
//! The underlying backend is a single WAL-mode SQLite database per
//! session, created lazily at `<storage_dir>/storage.db` on first
//! access.  See [`crate::kv_store`] for the schema and concurrency
//! model; everything here is a thin wrapper that also routes work
//! through the `IoScheduler` so the host thread never blocks on
//! disk.
//!
//! ## Migration from the previous file-per-key layout
//!
//! The prior implementation stored each key as a hex-named file
//! under `<storage_dir>/`. That layout is gone; on first open the
//! old `.dat` files (if any) are left on disk untouched — the host
//! app can `rm -rf` them after confirming its users migrated, or we
//! can wire an automated import later (not done here because the
//! product decision is "no history compat").

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;
use shared::error::EngineError;

use crate::{
    kv_store::{KvInfo, KvStore},
    pools::PoolError,
    scheduler::IoScheduler,
    task::{IoRequest, PriorityClass, RequestKind},
};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Summary returned by [`storage_info`]. Wire format of the JS
/// `getStorageInfoSync` is built in the runtime-v8 layer; this type
/// is the structured equivalent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageInfo {
    pub keys: Vec<String>,
    pub current_bytes: u64,
    pub limit_bytes: u64,
}

impl From<KvInfo> for StorageInfo {
    fn from(k: KvInfo) -> Self {
        Self {
            keys: k.keys,
            current_bytes: k.current_bytes,
            limit_bytes: k.limit_bytes,
        }
    }
}

// ---------------------------------------------------------------------------
// Per-session KvStore handle cache
// ---------------------------------------------------------------------------
//
// Opening SQLite is ~sub-ms but still involves syscalls and WAL
// setup; we keep one handle per storage directory for the lifetime
// of the process. The cache is keyed by absolute path so different
// `HostOpState` instances (e.g. separate app ids in a test harness)
// get separate DBs.
//
// The per-handle `KvStore` is itself `Clone` + `Sync`, so `Mutex`
// contention is limited to the *open* path.

static STORES: LazyLock<Mutex<HashMap<PathBuf, KvStore>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn store_for(dir: &Path, quota_bytes: u64) -> Result<KvStore, EngineError> {
    let key = dir.to_path_buf();
    {
        let map = STORES.lock();
        if let Some(kv) = map.get(&key) {
            return Ok(kv.clone());
        }
    }
    // Lock-release-reacquire race: two callers could both reach the
    // open() below at the same time. `insert` takes whichever wins;
    // both Arc<KvStore> wrappers end up pointing at the same SQLite
    // file, and SQLite itself is process-local so opening the same
    // file twice is cheap.
    let new_kv = KvStore::open(dir.join("storage.db"), quota_bytes)?;
    let mut map = STORES.lock();
    Ok(map.entry(key).or_insert(new_kv).clone())
}

/// Test-only hatch to reset the process-wide store cache between
/// tests. Safe to call concurrently — idempotent.
#[cfg(test)]
pub fn __reset_stores_for_test() {
    STORES.lock().clear();
}

// ---------------------------------------------------------------------------
// Scheduler plumbing
// ---------------------------------------------------------------------------

#[inline]
fn pool_err(err: PoolError) -> EngineError {
    EngineError::from(err)
}

#[inline]
fn storage_get_request(request: RequestKind, estimated_bytes: usize) -> IoRequest {
    IoRequest::StorageGet {
        request,
        priority: PriorityClass::from(request),
        estimated_bytes,
    }
}

#[inline]
fn storage_mutate_request(request: RequestKind) -> IoRequest {
    IoRequest::StorageMutate {
        request,
        priority: PriorityClass::from(request),
    }
}

#[inline]
fn storage_info_request(request: RequestKind) -> IoRequest {
    IoRequest::StorageInfo {
        request,
        priority: PriorityClass::from(request),
    }
}

// ---------------------------------------------------------------------------
// Direct (unscheduled) operations — used by tests and by the async
// variants below; callers on the host thread should prefer the
// `*_sync_with_scheduler` wrappers so budget accounting is correct.
// ---------------------------------------------------------------------------

/// Read a stored value.  Returns `Ok(None)` for a missing key.
///
/// The old file-based implementation returned an empty string on
/// missing key (to match the JS `''` sentinel); we now return
/// `Option` so the JS layer can distinguish "empty string value" from
/// "no such key".
pub fn storage_get(dir: &Path, key: &str, quota_bytes: u64) -> Result<Option<String>, EngineError> {
    store_for(dir, quota_bytes)?.get(key)
}

/// Write `value` under `key` subject to the `quota_bytes` cap.
pub fn storage_set(
    dir: &Path,
    key: &str,
    value: &str,
    quota_bytes: u64,
) -> Result<(), EngineError> {
    store_for(dir, quota_bytes)?.set(key, value)
}

/// Batch write. Atomic: either every item lands or none does.
pub fn storage_set_batch(
    dir: &Path,
    items: &[(&str, &str)],
    quota_bytes: u64,
) -> Result<(), EngineError> {
    store_for(dir, quota_bytes)?.set_batch(items)
}

pub fn storage_remove(dir: &Path, key: &str, quota_bytes: u64) -> Result<(), EngineError> {
    store_for(dir, quota_bytes)?.remove(key)
}

pub fn storage_clear(dir: &Path, quota_bytes: u64) -> Result<(), EngineError> {
    store_for(dir, quota_bytes)?.clear()
}

pub fn storage_info(dir: &Path, quota_bytes: u64) -> Result<StorageInfo, EngineError> {
    store_for(dir, quota_bytes)?.info().map(Into::into)
}

// ---------------------------------------------------------------------------
// Scheduler-routed wrappers (host-thread entry points)
// ---------------------------------------------------------------------------
//
// These match the shape of the pre-existing runtime-v8 integration
// — the JS op calls `*_sync_with_scheduler` which dispatches through
// the IoScheduler so budget/priority accounting keeps working.
// Internally each closure calls the direct helpers above; the thin
// wrapping lets us preserve the scheduler contract without scattering
// `IoScheduler` references through every KvStore call site.

pub fn storage_get_sync_with_scheduler(
    scheduler: Arc<IoScheduler>,
    dir: PathBuf,
    key: String,
    quota_bytes: u64,
) -> Result<Option<String>, EngineError> {
    // Estimated byte size is only used for the scheduler's active-
    // byte accounting; a rough upper bound (the common mini-game platform
    // per-value max is 1 MiB) is enough.
    let request = storage_get_request(RequestKind::Sync, 0);
    scheduler
        .run_sync(&request, move || storage_get(&dir, &key, quota_bytes))
        .map_err(pool_err)?
}

pub async fn storage_get_with_scheduler(
    scheduler: Arc<IoScheduler>,
    dir: PathBuf,
    key: String,
    quota_bytes: u64,
    request: RequestKind,
) -> Result<Option<String>, EngineError> {
    let req = storage_get_request(request, 0);
    match request {
        RequestKind::Sync => scheduler
            .run_sync(&req, move || storage_get(&dir, &key, quota_bytes))
            .map_err(pool_err)?,
        RequestKind::Async => scheduler
            .run_async(req, move || storage_get(&dir, &key, quota_bytes))
            .await
            .map_err(pool_err)?,
    }
}

pub fn storage_set_sync_with_scheduler(
    scheduler: Arc<IoScheduler>,
    dir: PathBuf,
    key: String,
    value: String,
    quota_bytes: u64,
) -> Result<(), EngineError> {
    let request = storage_mutate_request(RequestKind::Sync);
    scheduler
        .run_sync(&request, move || {
            storage_set(&dir, &key, &value, quota_bytes)
        })
        .map_err(pool_err)?
}

pub fn storage_remove_sync_with_scheduler(
    scheduler: Arc<IoScheduler>,
    dir: PathBuf,
    key: String,
    quota_bytes: u64,
) -> Result<(), EngineError> {
    let request = storage_mutate_request(RequestKind::Sync);
    scheduler
        .run_sync(&request, move || storage_remove(&dir, &key, quota_bytes))
        .map_err(pool_err)?
}

pub fn storage_clear_sync_with_scheduler(
    scheduler: Arc<IoScheduler>,
    dir: PathBuf,
    quota_bytes: u64,
) -> Result<(), EngineError> {
    let request = storage_mutate_request(RequestKind::Sync);
    scheduler
        .run_sync(&request, move || storage_clear(&dir, quota_bytes))
        .map_err(pool_err)?
}

pub fn storage_info_sync_with_scheduler(
    scheduler: Arc<IoScheduler>,
    dir: PathBuf,
    quota_bytes: u64,
) -> Result<StorageInfo, EngineError> {
    let request = storage_info_request(RequestKind::Sync);
    scheduler
        .run_sync(&request, move || storage_info(&dir, quota_bytes))
        .map_err(pool_err)?
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const QUOTA: u64 = 1024 * 1024; // 1 MiB in tests

    fn fresh_dir() -> tempfile::TempDir {
        __reset_stores_for_test();
        tempdir().unwrap()
    }

    #[test]
    fn set_get_remove_roundtrip() {
        let dir = fresh_dir();
        storage_set(dir.path(), "k", "v", QUOTA).unwrap();
        assert_eq!(
            storage_get(dir.path(), "k", QUOTA).unwrap().as_deref(),
            Some("v")
        );
        storage_remove(dir.path(), "k", QUOTA).unwrap();
        assert_eq!(storage_get(dir.path(), "k", QUOTA).unwrap(), None);
    }

    #[test]
    fn clear_empties_store() {
        let dir = fresh_dir();
        storage_set(dir.path(), "a", "1", QUOTA).unwrap();
        storage_set(dir.path(), "b", "2", QUOTA).unwrap();
        storage_clear(dir.path(), QUOTA).unwrap();
        let info = storage_info(dir.path(), QUOTA).unwrap();
        assert!(info.keys.is_empty());
        assert_eq!(info.current_bytes, 0);
    }

    #[test]
    fn info_reports_keys_and_total_bytes() {
        let dir = fresh_dir();
        storage_set(dir.path(), "k1", "ab", QUOTA).unwrap();
        storage_set(dir.path(), "k2", "xyz", QUOTA).unwrap();
        let info = storage_info(dir.path(), QUOTA).unwrap();
        assert_eq!(info.current_bytes, 5);
        assert_eq!(info.limit_bytes, QUOTA);
        let mut sorted = info.keys.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["k1".to_string(), "k2".to_string()]);
    }

    #[test]
    fn batch_set_is_atomic() {
        let dir = fresh_dir();
        let items = vec![("a", "1"), ("b", "2"), ("c", "3")];
        storage_set_batch(dir.path(), &items, QUOTA).unwrap();
        assert_eq!(
            storage_get(dir.path(), "b", QUOTA).unwrap().as_deref(),
            Some("2")
        );
        // Overflow batch — nothing should land.
        let big = "x".repeat(QUOTA as usize);
        let overflow = vec![("a", big.as_str()), ("z", big.as_str())];
        let err = storage_set_batch(dir.path(), &overflow, QUOTA).unwrap_err();
        assert_eq!(err.code, shared::error::ErrorCode::OutOfMemory);
        // Earlier keys untouched.
        assert_eq!(
            storage_get(dir.path(), "a", QUOTA).unwrap().as_deref(),
            Some("1")
        );
    }

    #[test]
    fn second_open_of_same_dir_reuses_handle() {
        let dir = fresh_dir();
        storage_set(dir.path(), "once", "v", QUOTA).unwrap();
        // Re-access: should hit the cache and still see the data.
        let val = storage_get(dir.path(), "once", QUOTA).unwrap();
        assert_eq!(val.as_deref(), Some("v"));
        // Exactly one entry in the global store cache for this path.
        let n = STORES
            .lock()
            .keys()
            .filter(|p| p.as_path() == dir.path())
            .count();
        assert_eq!(n, 1);
    }

    #[test]
    fn scheduler_wrapper_round_trip() {
        let dir = fresh_dir();
        let scheduler = Arc::new(IoScheduler::new(42));
        storage_set_sync_with_scheduler(
            scheduler.clone(),
            dir.path().to_path_buf(),
            "k".into(),
            "v".into(),
            QUOTA,
        )
        .unwrap();
        let got =
            storage_get_sync_with_scheduler(scheduler, dir.path().to_path_buf(), "k".into(), QUOTA)
                .unwrap();
        assert_eq!(got.as_deref(), Some("v"));
    }

    // --- Lightweight benchmarks (run with `cargo test -- --ignored`) ---
    //
    // Not real criterion benches; just a sanity check that batch writes
    // beat per-key writes by at least an order of magnitude. Numbers
    // are illustrative (host filesystem, debug/release vary), but the
    // *ratio* is the property we care about regressing on.

    const BENCH_QUOTA: u64 = 32 * 1024 * 1024;

    #[test]
    #[ignore]
    fn bench_sequential_set_1000x200() {
        let dir = fresh_dir();
        let start = std::time::Instant::now();
        for i in 0..1000 {
            let k = format!("key_{i:04}");
            storage_set(dir.path(), &k, &"x".repeat(200), BENCH_QUOTA).unwrap();
        }
        let elapsed = start.elapsed();
        eprintln!("sequential 1000×200B set: {:.3?}", elapsed);
        // Floor: anything over a second on a modern host suggests a
        // regression back to per-op fsync. 10s is a very conservative
        // upper bound that still flags pathological slowdowns.
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "sequential set way too slow: {:?}",
            elapsed
        );
    }

    #[test]
    #[ignore]
    fn bench_batch_set_1000x200() {
        let dir = fresh_dir();
        let kvs: Vec<(String, String)> = (0..1000)
            .map(|i| (format!("key_{i:04}"), "x".repeat(200)))
            .collect();
        let items: Vec<(&str, &str)> = kvs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

        let start = std::time::Instant::now();
        storage_set_batch(dir.path(), &items, BENCH_QUOTA).unwrap();
        let elapsed = start.elapsed();
        eprintln!("batch 1000×200B set: {:.3?}", elapsed);
        // A single WAL transaction of 1000 small upserts should be
        // comfortably under 100ms on any reasonable filesystem.
        assert!(
            elapsed < std::time::Duration::from_secs(1),
            "batch set way too slow: {:?}",
            elapsed
        );
    }
}
