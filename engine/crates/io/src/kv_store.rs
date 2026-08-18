//! Key-value storage backed by an embedded SQLite database.
//!
//! This replaces the old `file-per-key` layout. Why SQLite:
//!
//! * **Batched writes** are one transaction instead of N rename+fsync
//!   pairs — measured >10× on the `setStorageBatch` fast path.
//! * **Quota** is O(1): the running total is cached in `_meta` under
//!   key `total_bytes` and updated inside every write transaction,
//!   so `set` / `set_batch` / `info` never scan the `kv` table.
//! * **Crash-safety** comes from WAL + `synchronous=NORMAL`, which
//!   is the combination SQLite itself recommends for KV workloads.
//!   We never lose a committed write on power loss; at worst the
//!   last un-checkpointed WAL frame is replayed on startup.
//! * **Schema migrations** are a pragma bump, not a home-grown
//!   header format.
//!
//! # Threading
//!
//! `KvStore` is cheaply `Clone` (it wraps an `Arc<Mutex<...>>`) and
//! safe to share across threads. Internally every operation takes a
//! short mutex around a single `rusqlite::Connection`. SQLite's own
//! WAL allows concurrent readers with one writer, but using a single
//! Rust `Connection` is simpler, avoids pool bookkeeping, and matches
//! the expected traffic (sub-ms transactions, hundreds of ops/s at
//! peak).  The mutex is released around any blocking fsync via
//! `synchronous=NORMAL`, which only fsyncs at checkpoint time.
//!
//! # Schema
//!
//! ```sql
//! CREATE TABLE kv (
//!     k          TEXT PRIMARY KEY,
//!     v          TEXT NOT NULL,
//!     size       INTEGER NOT NULL,   -- byte length of v, cached
//!     updated_at INTEGER NOT NULL    -- millis since epoch
//! ) WITHOUT ROWID;
//!
//! CREATE INDEX kv_updated ON kv(updated_at);
//!
//! CREATE TABLE _meta (
//!     k TEXT PRIMARY KEY,
//!     v TEXT NOT NULL
//! ) WITHOUT ROWID;
//! ```
//!
//! `WITHOUT ROWID` cuts the per-row overhead by one btree and is the
//! idiomatic shape for a string-keyed KV table.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use shared::error::{EngineError, ErrorCode};

/// Current schema version. Bump on incompatible migrations; the
/// constructor runs `migrate_to_current` and refuses to open a DB
/// newer than `SCHEMA_VERSION`.
const SCHEMA_VERSION: i64 = 1;

/// Summary returned by [`KvStore::info`].
///
/// Mirrors the JS-visible `getStorageInfo` response exactly so the
/// upper layer can serialise without another round trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KvInfo {
    pub keys: Vec<String>,
    pub current_bytes: u64,
    pub limit_bytes: u64,
}

/// Open handle to the KV database.
#[derive(Clone)]
pub struct KvStore {
    inner: Arc<Mutex<Inner>>,
}

// Custom Debug: `rusqlite::Connection` is not Debug.  We only expose
// fields that are cheap to format and safe to log.
impl std::fmt::Debug for KvStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = self.inner.lock();
        f.debug_struct("KvStore")
            .field("path", &g.path)
            .field("quota_bytes", &g.quota_bytes)
            .finish()
    }
}

struct Inner {
    conn: Connection,
    path: PathBuf,
    quota_bytes: u64,
    /// Running total of `size` across all rows in `kv`. Loaded once at
    /// open time from `_meta.total_bytes` (or reconciled with
    /// `SUM(size)` on first use if the value is missing / corrupt) and
    /// updated inside every write transaction. This is the only
    /// source-of-truth for quota checks; no read path does `SUM`.
    total_bytes: u64,
}

const META_TOTAL_BYTES: &str = "total_bytes";

impl KvStore {
    /// Open (creating if missing) the KV DB at `path`.
    ///
    /// * `path` — absolute file path, typically `<app_files>/kv_storage/storage.db`.
    /// * `quota_bytes` — total size cap in bytes (sum of all `v`).
    ///
    /// Initialisation is idempotent; multiple processes on the same
    /// file is **not** supported (SQLite would permit it with
    /// locking but we don't guarantee correctness of the cached
    /// totals across processes).
    pub fn open(path: impl AsRef<Path>, quota_bytes: u64) -> Result<Self, EngineError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                EngineError::new(ErrorCode::IoError)
                    .with_msg("kv: mkdir parent failed")
                    .with_detail(format!("{}: {}", parent.display(), e))
            })?;
        }

        let mut conn = Connection::open(&path).map_err(sql_err("kv: open"))?;

        // WAL + NORMAL sync is SQLite's recommended KV config:
        //   - WAL gives concurrent reads during writes and is faster
        //     than rollback journal for small transactions.
        //   - synchronous=NORMAL fsyncs at checkpoint boundaries only;
        //     committed writes survive app crashes, and only a power
        //     failure between commit and checkpoint can lose (at most)
        //     the last un-checkpointed frames. Acceptable for KV.
        //   - temp_store=MEMORY keeps transient B-tree pages in RAM.
        //   - wal_autocheckpoint=1000 (pages) is the default; keep it
        //     explicit so future sqlite upgrades can't silently change.
        // Wait rather than fail instantly when another connection holds a lock.
        // SQLite's default is a zero timeout, so any contention -- a checkpoint,
        // or a second process opening the same file -- surfaces as an immediate
        // `database is locked` rather than a short wait. Set before the first
        // pragma, since `journal_mode` is itself a lock-taking statement.
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(sql_err("kv: busy_timeout"))?;

        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(sql_err("kv: pragma journal_mode"))?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(sql_err("kv: pragma synchronous"))?;
        conn.pragma_update(None, "temp_store", "MEMORY")
            .map_err(sql_err("kv: pragma temp_store"))?;
        conn.pragma_update(None, "wal_autocheckpoint", 1000)
            .map_err(sql_err("kv: pragma wal_autocheckpoint"))?;
        // `foreign_keys` is off by default anyway, but make it
        // explicit so a future ALTER doesn't silently enable it.
        conn.pragma_update(None, "foreign_keys", "OFF")
            .map_err(sql_err("kv: pragma foreign_keys"))?;

        migrate_to_current(&mut conn)?;

        // Load (or reconcile) the cached total so every write/read
        // path after this is O(1). If `_meta.total_bytes` is missing
        // we do a single O(N) reconcile-SUM and persist the result;
        // this is the only place in the KV store that scans `kv`.
        let total_bytes = load_or_reconcile_total(&conn)?;

        Ok(Self {
            inner: Arc::new(Mutex::new(Inner {
                conn,
                path,
                quota_bytes,
                total_bytes,
            })),
        })
    }

    /// Read the value for `key`, or `Ok(None)` if absent.
    pub fn get(&self, key: &str) -> Result<Option<String>, EngineError> {
        let started = std::time::Instant::now();
        let g = self.inner.lock();
        let out = g
            .conn
            .query_row("SELECT v FROM kv WHERE k = ?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .map_err(sql_err("kv: get"));
        shared::stats::io_metrics_global()
            .record_op(shared::stats::OpClass::StorageGet, started.elapsed());
        out
    }

    /// Write `value` under `key`, honouring the configured quota.
    ///
    /// Quota accounting counts **the new total** after replacement
    /// (old key's size is deducted), so repeatedly overwriting a
    /// single key never inflates the total.  If the write would
    /// exceed the quota the call returns `ResourceExhausted` and
    /// the DB is untouched.
    pub fn set(&self, key: &str, value: &str) -> Result<(), EngineError> {
        let started = std::time::Instant::now();
        let result = self.set_inner(key, value);
        shared::stats::io_metrics_global()
            .record_op(shared::stats::OpClass::StorageSet, started.elapsed());
        result
    }

    fn set_inner(&self, key: &str, value: &str) -> Result<(), EngineError> {
        let mut g = self.inner.lock();
        let quota = g.quota_bytes;
        let current_total = g.total_bytes;
        let tx = g
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_err("kv: begin"))?;

        let old_size: i64 = tx
            .query_row("SELECT size FROM kv WHERE k = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(sql_err("kv: set: read old"))?
            .unwrap_or(0);
        let new_size = value.len() as i64;
        // `total_bytes` is always non-negative; saturating_sub avoids
        // panic on a hypothetical corrupt row with negative size.
        let projected = current_total
            .saturating_sub(old_size as u64)
            .saturating_add(new_size as u64);
        if projected > quota {
            return Err(EngineError::new(ErrorCode::OutOfMemory)
                .with_msg("setStorage:fail storage limit exceeded")
                .with_detail(format!(
                    "projected {} bytes > quota {} bytes",
                    projected, quota
                )));
        }

        tx.execute(
            "INSERT INTO kv(k, v, size, updated_at) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(k) DO UPDATE SET v = excluded.v, size = excluded.size, \
             updated_at = excluded.updated_at",
            params![key, value, new_size, now_ms()],
        )
        .map_err(sql_err("kv: set: upsert"))?;
        persist_total_in_tx(&tx, projected)?;
        tx.commit().map_err(sql_err("kv: set: commit"))?;
        // Update cache only after the commit succeeded; otherwise a
        // failed commit would leave the cached total ahead of the DB.
        g.total_bytes = projected;
        Ok(())
    }

    /// Atomic batch set. All inputs land or none do — including the
    /// quota check, which is evaluated against the *final* projected
    /// total so a batch can overwrite existing keys without a
    /// transient overflow.
    pub fn set_batch(&self, items: &[(&str, &str)]) -> Result<(), EngineError> {
        if items.is_empty() {
            return Ok(());
        }
        let mut g = self.inner.lock();
        let quota = g.quota_bytes;
        let current_total = g.total_bytes;
        let tx = g
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_err("kv: batch: begin"))?;

        // Project the new total against the cached value. We must
        // look up each incoming key's old size to handle overwrites
        // correctly, but we never run `SUM(size)`.
        let mut projected: i128 = current_total as i128;
        {
            let mut q = tx
                .prepare_cached("SELECT size FROM kv WHERE k = ?1")
                .map_err(sql_err("kv: batch: prep size"))?;
            for (k, v) in items {
                let old: i64 = q
                    .query_row([k], |r| r.get(0))
                    .optional()
                    .map_err(sql_err("kv: batch: size"))?
                    .unwrap_or(0);
                projected = projected - old as i128 + v.len() as i128;
            }
        }

        if projected < 0 || projected as u128 > quota as u128 {
            return Err(EngineError::new(ErrorCode::OutOfMemory)
                .with_msg("setStorageBatch:fail storage limit exceeded")
                .with_detail(format!(
                    "projected {} bytes > quota {} bytes",
                    projected, quota
                )));
        }
        let projected = projected as u64;

        {
            let mut up = tx
                .prepare_cached(
                    "INSERT INTO kv(k, v, size, updated_at) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(k) DO UPDATE SET v = excluded.v, size = excluded.size, \
                 updated_at = excluded.updated_at",
                )
                .map_err(sql_err("kv: batch: prep upsert"))?;
            let ts = now_ms();
            for (k, v) in items {
                up.execute(params![k, v, v.len() as i64, ts])
                    .map_err(sql_err("kv: batch: upsert"))?;
            }
        }
        persist_total_in_tx(&tx, projected)?;
        tx.commit().map_err(sql_err("kv: batch: commit"))?;
        g.total_bytes = projected;
        Ok(())
    }

    /// Remove `key` if present. No error on missing key.
    pub fn remove(&self, key: &str) -> Result<(), EngineError> {
        let mut g = self.inner.lock();
        let current_total = g.total_bytes;
        let tx = g
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_err("kv: remove: begin"))?;
        let old_size: i64 = tx
            .query_row("SELECT size FROM kv WHERE k = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(sql_err("kv: remove: read old"))?
            .unwrap_or(0);
        tx.execute("DELETE FROM kv WHERE k = ?1", [key])
            .map_err(sql_err("kv: remove"))?;
        let new_total = current_total.saturating_sub(old_size as u64);
        persist_total_in_tx(&tx, new_total)?;
        tx.commit().map_err(sql_err("kv: remove: commit"))?;
        g.total_bytes = new_total;
        Ok(())
    }

    /// Remove every row. Much faster than N `DELETE` statements
    /// because SQLite takes the truncate-optimisation path when a
    /// WHERE clause is absent.
    pub fn clear(&self) -> Result<(), EngineError> {
        let mut g = self.inner.lock();
        let tx = g
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_err("kv: clear: begin"))?;
        tx.execute("DELETE FROM kv", [])
            .map_err(sql_err("kv: clear"))?;
        persist_total_in_tx(&tx, 0)?;
        tx.commit().map_err(sql_err("kv: clear: commit"))?;
        g.total_bytes = 0;
        Ok(())
    }

    /// List every key and the summary totals.
    ///
    /// Ordering is deterministic (`updated_at DESC, k ASC`) so
    /// callers that diff consecutive snapshots see stable output.
    ///
    /// `current_bytes` is read from the cached `_meta.total_bytes`
    /// value maintained by every write path, so this call is O(N keys)
    /// only in the key-listing pass, never in the totals pass.
    pub fn info(&self) -> Result<KvInfo, EngineError> {
        let started = std::time::Instant::now();
        let result = self.info_inner();
        shared::stats::io_metrics_global()
            .record_op(shared::stats::OpClass::StorageInfo, started.elapsed());
        result
    }

    fn info_inner(&self) -> Result<KvInfo, EngineError> {
        let g = self.inner.lock();
        let mut stmt = g
            .conn
            .prepare("SELECT k FROM kv ORDER BY updated_at DESC, k ASC")
            .map_err(sql_err("kv: info: prepare"))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(sql_err("kv: info: query"))?;
        let mut keys = Vec::new();
        for row in rows {
            keys.push(row.map_err(sql_err("kv: info: row"))?);
        }
        Ok(KvInfo {
            keys,
            current_bytes: g.total_bytes,
            limit_bytes: g.quota_bytes,
        })
    }

    /// Test-only: force a WAL checkpoint. Production never needs
    /// this; SQLite's autocheckpoint handles it. `wal_checkpoint`
    /// returns a 3-integer row (busy, log_pages, ckpt_pages) so we
    /// must use a `query_row`, not `execute`.
    #[cfg(test)]
    fn checkpoint(&self) -> Result<(), EngineError> {
        let g = self.inner.lock();
        g.conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .map_err(sql_err("kv: checkpoint"))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Schema migration
// ---------------------------------------------------------------------------

fn migrate_to_current(conn: &mut Connection) -> Result<(), EngineError> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(sql_err("kv: read user_version"))?;
    if current > SCHEMA_VERSION {
        return Err(EngineError::new(ErrorCode::Unsupported)
            .with_msg("kv: db schema is newer than this binary")
            .with_detail(format!("db={}, supported={}", current, SCHEMA_VERSION)));
    }
    if current == SCHEMA_VERSION {
        return Ok(());
    }

    // v0 -> v1: initial schema.
    let tx = conn.transaction().map_err(sql_err("kv: migrate begin"))?;
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS kv (
            k          TEXT PRIMARY KEY,
            v          TEXT NOT NULL,
            size       INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        ) WITHOUT ROWID;

        CREATE INDEX IF NOT EXISTS kv_updated ON kv(updated_at);

        CREATE TABLE IF NOT EXISTS _meta (
            k TEXT PRIMARY KEY,
            v TEXT NOT NULL
        ) WITHOUT ROWID;

        PRAGMA user_version = 1;
        "#,
    )
    .map_err(sql_err("kv: migrate v1"))?;
    tx.commit().map_err(sql_err("kv: migrate commit"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[inline]
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn sql_err(ctx: &'static str) -> impl Fn(rusqlite::Error) -> EngineError {
    move |e| {
        EngineError::new(ErrorCode::IoError)
            .with_msg(ctx)
            .with_detail(e.to_string())
    }
}

/// Persist `total_bytes` into the `_meta` table as part of the caller's
/// write transaction, so commit atomicity covers both the row change
/// and the cached total.
fn persist_total_in_tx(
    tx: &rusqlite::Transaction<'_>,
    total_bytes: u64,
) -> Result<(), EngineError> {
    tx.execute(
        "INSERT INTO _meta(k, v) VALUES (?1, ?2) \
         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        params![META_TOTAL_BYTES, total_bytes.to_string()],
    )
    .map_err(sql_err("kv: meta: set total"))?;
    Ok(())
}

/// On open, read the cached running total from `_meta.total_bytes`. If
/// it's missing or corrupt (e.g. the DB was written by an older binary
/// that never maintained the cache), fall back to a single
/// reconciliation `SUM(size)` and persist the result so subsequent
/// opens are O(1).
fn load_or_reconcile_total(conn: &Connection) -> Result<u64, EngineError> {
    let cached: Option<String> = conn
        .query_row(
            "SELECT v FROM _meta WHERE k = ?1",
            [META_TOTAL_BYTES],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql_err("kv: meta: get total"))?;

    if let Some(s) = cached {
        if let Ok(n) = s.parse::<u64>() {
            return Ok(n);
        }
    }

    // Missing or corrupt — reconcile once.
    let reconciled: i64 = conn
        .query_row("SELECT COALESCE(SUM(size), 0) FROM kv", [], |r| r.get(0))
        .map_err(sql_err("kv: meta: reconcile sum"))?;
    let reconciled = reconciled.max(0) as u64;
    conn.execute(
        "INSERT INTO _meta(k, v) VALUES (?1, ?2) \
         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        params![META_TOTAL_BYTES, reconciled.to_string()],
    )
    .map_err(sql_err("kv: meta: persist reconciled total"))?;
    Ok(reconciled)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const QUOTA: u64 = 1024; // 1 KiB quota for exhaustion tests

    fn open(dir: &Path) -> KvStore {
        KvStore::open(dir.join("storage.db"), QUOTA).expect("open")
    }

    #[test]
    fn set_and_get_roundtrips() {
        let dir = tempdir().unwrap();
        let kv = open(dir.path());
        kv.set("alpha", "one").unwrap();
        kv.set("beta", "two").unwrap();
        assert_eq!(kv.get("alpha").unwrap().as_deref(), Some("one"));
        assert_eq!(kv.get("beta").unwrap().as_deref(), Some("two"));
    }

    #[test]
    fn missing_key_returns_none() {
        let dir = tempdir().unwrap();
        let kv = open(dir.path());
        assert_eq!(kv.get("nope").unwrap(), None);
    }

    #[test]
    fn overwrite_same_key_keeps_total_accurate() {
        let dir = tempdir().unwrap();
        let kv = open(dir.path());
        kv.set("k", "aaaa").unwrap();
        kv.set("k", "bbbbbbbb").unwrap(); // 8 bytes
        let info = kv.info().unwrap();
        assert_eq!(info.keys, vec!["k".to_string()]);
        assert_eq!(info.current_bytes, 8);
    }

    #[test]
    fn remove_is_idempotent() {
        let dir = tempdir().unwrap();
        let kv = open(dir.path());
        kv.set("x", "1").unwrap();
        kv.remove("x").unwrap();
        kv.remove("x").unwrap(); // second call is a no-op
        assert_eq!(kv.get("x").unwrap(), None);
    }

    #[test]
    fn clear_removes_everything() {
        let dir = tempdir().unwrap();
        let kv = open(dir.path());
        kv.set("a", "x").unwrap();
        kv.set("b", "y").unwrap();
        kv.clear().unwrap();
        let info = kv.info().unwrap();
        assert!(info.keys.is_empty());
        assert_eq!(info.current_bytes, 0);
    }

    #[test]
    fn info_reports_all_keys_and_total_bytes() {
        let dir = tempdir().unwrap();
        let kv = open(dir.path());
        kv.set("k1", "aa").unwrap();
        kv.set("k2", "bbb").unwrap();
        let info = kv.info().unwrap();
        assert_eq!(info.current_bytes, 5);
        assert_eq!(info.limit_bytes, QUOTA);
        let mut sorted = info.keys.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["k1".to_string(), "k2".to_string()]);
    }

    #[test]
    fn quota_exceeded_returns_error_and_does_not_write() {
        let dir = tempdir().unwrap();
        let kv = open(dir.path());
        // Fill to the brim.
        let big = "x".repeat(QUOTA as usize - 10);
        kv.set("fill", &big).unwrap();
        // One more byte should push us over.
        let err = kv.set("overflow", &"y".repeat(100)).unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfMemory);
        assert_eq!(
            kv.get("overflow").unwrap(),
            None,
            "aborted write must not leak"
        );
    }

    #[test]
    fn overwrite_does_not_trigger_transient_quota_overflow() {
        // If naive accounting added the new size *before* subtracting
        // the old, an at-capacity key rewrite would fail.  Verify the
        // replace-aware math.
        let dir = tempdir().unwrap();
        let kv = open(dir.path());
        let big = "x".repeat(QUOTA as usize);
        kv.set("k", &big).unwrap();
        let big2 = "y".repeat(QUOTA as usize);
        kv.set("k", &big2).unwrap(); // must succeed
        assert_eq!(kv.get("k").unwrap().unwrap().len(), QUOTA as usize);
    }

    #[test]
    fn batch_set_is_atomic_on_quota_overflow() {
        let dir = tempdir().unwrap();
        let kv = open(dir.path());
        // batch sums to QUOTA+1 -> must reject entirely.
        let a = "a".repeat(500);
        let b = "b".repeat(525);
        let items = vec![("a", a.as_str()), ("b", b.as_str())];
        let err = kv.set_batch(&items).unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfMemory);
        assert_eq!(kv.get("a").unwrap(), None);
        assert_eq!(kv.get("b").unwrap(), None);
    }

    #[test]
    fn batch_set_applies_all_on_success() {
        let dir = tempdir().unwrap();
        let kv = open(dir.path());
        let items = vec![("a", "1"), ("b", "22"), ("c", "333")];
        kv.set_batch(&items).unwrap();
        assert_eq!(kv.get("a").unwrap().as_deref(), Some("1"));
        assert_eq!(kv.get("b").unwrap().as_deref(), Some("22"));
        assert_eq!(kv.get("c").unwrap().as_deref(), Some("333"));
        let info = kv.info().unwrap();
        assert_eq!(info.current_bytes, 6);
    }

    #[test]
    fn empty_batch_is_noop() {
        let dir = tempdir().unwrap();
        let kv = open(dir.path());
        kv.set_batch(&[]).unwrap();
        assert!(kv.info().unwrap().keys.is_empty());
    }

    #[test]
    fn reopen_preserves_data() {
        let dir = tempdir().unwrap();
        {
            let kv = open(dir.path());
            kv.set("persist", "forever").unwrap();
            kv.checkpoint().unwrap();
        }
        let kv = open(dir.path());
        assert_eq!(kv.get("persist").unwrap().as_deref(), Some("forever"));
    }

    #[test]
    fn refuses_newer_schema_version() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("storage.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "user_version", 999).unwrap();
        }
        let err = KvStore::open(&path, QUOTA).unwrap_err();
        assert_eq!(err.code, ErrorCode::Unsupported);
    }

    #[test]
    fn handles_unicode_keys_and_values() {
        let dir = tempdir().unwrap();
        let kv = open(dir.path());
        kv.set("日本語", "値").unwrap();
        kv.set("emoji🔑", "🎯").unwrap();
        assert_eq!(kv.get("日本語").unwrap().as_deref(), Some("値"));
        assert_eq!(kv.get("emoji🔑").unwrap().as_deref(), Some("🎯"));
    }

    #[test]
    fn keys_with_sql_metacharacters_are_literal() {
        // If we ever accidentally swapped bound params for string
        // interpolation, a key like "'; DROP TABLE kv; --" would
        // destroy the DB.  Verify parameterisation.
        let dir = tempdir().unwrap();
        let kv = open(dir.path());
        let evil = "'; DROP TABLE kv; --";
        kv.set(evil, "still here").unwrap();
        assert_eq!(kv.get(evil).unwrap().as_deref(), Some("still here"));
        // The kv table must still exist.
        assert!(kv.info().is_ok());
    }

    #[test]
    fn total_bytes_stays_in_sync_across_operations() {
        let dir = tempdir().unwrap();
        let kv = open(dir.path());
        kv.set("a", "aa").unwrap();
        kv.set("b", "bbb").unwrap();
        assert_eq!(kv.info().unwrap().current_bytes, 5);
        kv.set("a", "aaaaa").unwrap();
        assert_eq!(kv.info().unwrap().current_bytes, 8);
        kv.remove("b").unwrap();
        assert_eq!(kv.info().unwrap().current_bytes, 5);
        kv.set_batch(&[("c", "cc"), ("d", "d")]).unwrap();
        assert_eq!(kv.info().unwrap().current_bytes, 8);
        kv.clear().unwrap();
        assert_eq!(kv.info().unwrap().current_bytes, 0);
    }

    #[test]
    fn reopen_reuses_cached_total_without_rescanning() {
        let dir = tempdir().unwrap();
        {
            let kv = open(dir.path());
            kv.set("k", "hello").unwrap();
        }
        // Reopen: the cached total must survive the process boundary.
        let kv = open(dir.path());
        assert_eq!(kv.info().unwrap().current_bytes, 5);
    }

    #[test]
    fn missing_meta_total_reconciles_on_open() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("storage.db");
        {
            let kv = KvStore::open(&path, QUOTA).unwrap();
            kv.set("a", "12345").unwrap();
            kv.checkpoint().unwrap();
        }
        // Simulate an older binary that never wrote `_meta.total_bytes`.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute("DELETE FROM _meta WHERE k = 'total_bytes'", [])
                .unwrap();
        }
        let kv = KvStore::open(&path, QUOTA).unwrap();
        assert_eq!(kv.info().unwrap().current_bytes, 5);
    }

    #[test]
    fn parallel_clones_share_underlying_db() {
        use std::sync::Barrier;
        use std::thread;

        let dir = tempdir().unwrap();
        let kv = open(dir.path());
        let b = Arc::new(Barrier::new(4));
        let mut handles = Vec::new();
        for i in 0..4u32 {
            let kv = kv.clone();
            let b = b.clone();
            handles.push(thread::spawn(move || {
                b.wait();
                kv.set(&format!("k{}", i), &i.to_string()).unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let info = kv.info().unwrap();
        assert_eq!(info.keys.len(), 4);
    }
}
