//! Disk-backed V8 code cache for faster module loading.
//!
//! Persists compiled JS bytecode to `<app_cache>/migo_code_cache/` so that
//! subsequent launches skip V8 parse+compile for both game modules and
//! engine extension JS.
//!
//! Cache invalidation:
//! - Source hash mismatch -> stale entry deleted, recompile
//! - V8 version change   -> entire cache dir cleared
//! - Max 32 MB total      -> LRU eviction by mtime

use std::{
    borrow::Cow,
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    rc::Rc,
};

use deno_core::{ModuleSourceCode, ModuleSpecifier, SourceCodeCacheInfo};

/// Maximum total cache size in bytes (32 MB).
const MAX_CACHE_SIZE: u64 = 32 * 1024 * 1024;

/// V8 code cache backed by the filesystem.
///
/// Cache key = `hash(source_bytes, v8_version)`.
/// File layout:
/// ```text
/// <cache_dir>/
///   v8_version.txt     # V8 version marker for bulk invalidation
///   <hex_hash>.bin     # compiled bytecode
/// ```
pub(crate) struct DiskCodeCache {
    cache_dir: PathBuf,
    v8_version: &'static str,
    /// Incrementally tracked total size of .bin files in the cache directory.
    /// Initialized with a single scan at construction, then maintained on
    /// each write/eviction to avoid O(N) scans per `set()`.
    current_size: std::cell::Cell<u64>,
}

impl DiskCodeCache {
    pub fn new(app_cache_dir: &Path) -> Self {
        let cache_dir = app_cache_dir.join("migo_code_cache");
        let v8_version = deno_core::v8::V8::get_version();

        let cache = Self {
            cache_dir,
            v8_version,
            current_size: std::cell::Cell::new(0),
        };
        cache.ensure_dir_and_check_version();
        // Initial scan to populate current_size.
        cache.current_size.set(cache.scan_total_size());
        cache
    }

    /// Ensure cache dir exists and invalidate if V8 version changed.
    fn ensure_dir_and_check_version(&self) {
        let _ = fs::create_dir_all(&self.cache_dir);

        let version_file = self.cache_dir.join("v8_version.txt");
        let stored_version = fs::read_to_string(&version_file).unwrap_or_default();

        if stored_version.trim() != self.v8_version {
            tracing::info!(
                "V8 version changed ({} -> {}), clearing code cache",
                stored_version.trim(),
                self.v8_version
            );
            self.clear_all();
            let _ = fs::create_dir_all(&self.cache_dir);
            let _ = fs::write(&version_file, self.v8_version);
        }
    }

    /// Compute a u64 hash from source bytes, incorporating V8 version.
    pub fn compute_hash(&self, source: &[u8]) -> u64 {
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        self.v8_version.hash(&mut hasher);
        hasher.finish()
    }

    /// Get cached bytecode for the given source hash.
    pub fn get(&self, hash: u64) -> Option<Vec<u8>> {
        let path = self.hash_path(hash);
        match fs::read(&path) {
            Ok(data) if !data.is_empty() => Some(data),
            _ => None,
        }
    }

    /// Save compiled bytecode for the given source hash.
    pub fn set(&self, hash: u64, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let path = self.hash_path(hash);
        // Account for replacing an existing file.
        let old_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if fs::write(&path, data).is_ok() {
            let new_size = data.len() as u64;
            let cur = self.current_size.get();
            self.current_size.set(cur.saturating_sub(old_size) + new_size);
            self.evict_if_needed();
        }
    }

    /// Clear the entire cache directory.
    pub fn clear_all(&self) {
        let _ = fs::remove_dir_all(&self.cache_dir);
        self.current_size.set(0);
    }

    fn hash_path(&self, hash: u64) -> PathBuf {
        self.cache_dir.join(format!("{:016x}.bin", hash))
    }

    /// Scan the cache directory and return the total size of .bin files.
    fn scan_total_size(&self) -> u64 {
        let entries = match fs::read_dir(&self.cache_dir) {
            Ok(e) => e,
            Err(_) => return 0,
        };
        let mut total: u64 = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("bin") {
                continue;
            }
            total += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
        total
    }

    /// Evict oldest cache files if total size exceeds MAX_CACHE_SIZE.
    ///
    /// O(1) check in the common case (under limit). Only does a full
    /// directory scan + sort when eviction is actually needed.
    fn evict_if_needed(&self) {
        if self.current_size.get() <= MAX_CACHE_SIZE {
            return;
        }

        // Over limit — do a full scan to get accurate sizes + mtimes for LRU eviction.
        let entries = match fs::read_dir(&self.cache_dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
        let mut total_size: u64 = 0;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("bin") {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                let size = meta.len();
                let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                total_size += size;
                files.push((path, size, mtime));
            }
        }

        if total_size <= MAX_CACHE_SIZE {
            // Incremental tracker drifted — correct it.
            self.current_size.set(total_size);
            return;
        }

        // Sort by mtime ascending (oldest first)
        files.sort_by_key(|(_, _, mtime)| *mtime);

        for (path, size, _) in &files {
            if total_size <= MAX_CACHE_SIZE {
                break;
            }
            if fs::remove_file(path).is_ok() {
                total_size = total_size.saturating_sub(*size);
            }
        }

        // Update tracker to post-eviction total.
        self.current_size.set(total_size);
    }
}

/// Shared handle to DiskCodeCache, usable from both ModuleLoader and
/// ExtCodeCache (both Rc-based, single-threaded on Host thread).
pub(crate) type SharedCodeCache = Rc<DiskCodeCache>;

/// Create a new shared code cache.
pub(crate) fn create_code_cache(app_cache_dir: &Path) -> SharedCodeCache {
    Rc::new(DiskCodeCache::new(app_cache_dir))
}

/// Adapter implementing deno_core's `ExtCodeCache` trait for caching
/// built-in extension JS (02_async.js, 98_global_scope.js, etc.).
pub(crate) struct ExtCodeCacheAdapter {
    inner: SharedCodeCache,
}

impl ExtCodeCacheAdapter {
    pub fn new(cache: SharedCodeCache) -> Rc<Self> {
        Rc::new(Self { inner: cache })
    }
}

impl deno_core::ExtCodeCache for ExtCodeCacheAdapter {
    fn get_code_cache_info(
        &self,
        _specifier: &ModuleSpecifier,
        code: &ModuleSourceCode,
        _esm: bool,
    ) -> SourceCodeCacheInfo {
        let source_bytes = code.as_bytes();
        let hash = self.inner.compute_hash(source_bytes);
        let data = self.inner.get(hash).map(|v| Cow::Owned(v));
        SourceCodeCacheInfo { hash, data }
    }

    fn code_cache_ready(
        &self,
        _specifier: ModuleSpecifier,
        hash: u64,
        code_cache: &[u8],
        _esm: bool,
    ) {
        self.inner.set(hash, code_cache);
    }
}
