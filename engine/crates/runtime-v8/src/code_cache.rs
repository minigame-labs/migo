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
//!
//! # Scope: one cache per directory, not one per Session
//!
//! The directory comes from `MigoEngineConfig.code_cache_dir`, which is per Engine,
//! so two Sessions on one Engine are handed the same directory. Section 6.5 says that
//! is right -- compiled bytecode for a given source is the same bytes whichever
//! Session asked for it, and the key is the source's own hash, so two games loading
//! one module should hold one copy.
//!
//! What was wrong was the accounting, and in the shape Section 6.4 defect 4 names:
//! the budget's denominator was an instance and its numerator a directory. Each Host
//! built its own `DiskCodeCache`, each scanned the directory once and then tracked its
//! own writes, and neither could see the other's. Three consequences, all from that
//! one mismatch: the 32 MB ceiling admitted N x 32 MB; one Session's eviction deleted
//! files another Session's counter still claimed, so that counter over-counted and
//! over-evicted; and two Sessions could write one path at once.
//!
//! So the directory owns the cache: [`create_code_cache`] hands back the instance
//! that directory already has, and its lock is what orders one Session's write
//! against another's read.
//!
//! **Not addressed, and stated rather than implied.** Two *processes* pointed at one
//! directory still get a budget each, because nothing here takes an OS-level lock on
//! the directory. The counter's drift correction in [`DiskCodeCache::evict_if_needed`]
//! is what keeps that bounded rather than unbounded: a scan that finds the directory
//! already under the ceiling replaces the tracked figure with the measured one.

use std::{
    borrow::Cow,
    collections::{HashMap, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, LazyLock, Weak},
};

use deno_core::{ModuleSourceCode, ModuleSpecifier, SourceCodeCacheInfo};
use parking_lot::{Mutex, RwLock};

/// Maximum total cache size in bytes (32 MB).
const MAX_CACHE_SIZE: u64 = 32 * 1024 * 1024;

/// The cache each directory has, if any Session still holds it.
///
/// `Weak` rather than `Arc`: the last Session to let go of a directory drops its
/// cache, and the next Session to ask for it builds a new one -- including the
/// opening scan, which is how a counter that has been away comes back correct.
static CACHES: LazyLock<Mutex<HashMap<PathBuf, Weak<DiskCodeCache>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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
    /// The directory's lock, holding the total size of its `.bin` files.
    ///
    /// Two things at once, deliberately. The bytes are the budget's numerator, and
    /// they are tracked incrementally because a scan per write is O(N) in a directory
    /// a game start walks tens of times. The lock is what orders a Session reading an
    /// entry against a Session replacing it -- `get` needs only that, so it takes the
    /// guard shared and concurrent module loads do not queue behind each other.
    directory: RwLock<u64>,
}

impl DiskCodeCache {
    /// Private, because a cache the registry does not know about is a second budget
    /// over the same directory, which is the defect this module exists to prevent.
    fn new(cache_dir: PathBuf) -> Self {
        let v8_version = deno_core::v8::V8::get_version();

        let cache = Self {
            cache_dir,
            v8_version,
            directory: RwLock::new(0),
        };
        cache.ensure_dir_and_check_version();
        // Opening scan: what is already on disk counts against the ceiling.
        let measured = cache.scan_total_size();
        *cache.directory.write() = measured;
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
        // Shared, so two Sessions starting at once read their modules concurrently.
        // A Session replacing this entry holds the guard exclusively, so what is read
        // here is never half of a write.
        let _directory = self.directory.read();
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
        let mut tracked = self.directory.write();
        // Account for replacing an existing file.
        let old_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if fs::write(&path, data).is_ok() {
            let new_size = data.len() as u64;
            *tracked = tracked.saturating_sub(old_size) + new_size;
            self.evict_if_needed(&mut tracked);
        }
    }

    /// Clear the entire cache directory.
    fn clear_all(&self) {
        let mut tracked = self.directory.write();
        let _ = fs::remove_dir_all(&self.cache_dir);
        *tracked = 0;
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
    fn evict_if_needed(&self, tracked: &mut u64) {
        if *tracked <= MAX_CACHE_SIZE {
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
            // The tracked figure drifted above the measured one, which in one process
            // means only that another process shares this directory. Measured wins.
            *tracked = total_size;
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
        *tracked = total_size;
    }
}

/// Shared handle to the directory's `DiskCodeCache`, usable from every Session's
/// ModuleLoader and ExtCodeCache.
pub(crate) type SharedCodeCache = Arc<DiskCodeCache>;

/// The cache for `app_cache_dir`, built if this is the first Session to ask.
pub(crate) fn create_code_cache(app_cache_dir: &Path) -> SharedCodeCache {
    let cache_dir = code_cache_dir(app_cache_dir);

    let mut caches = CACHES.lock();
    if let Some(live) = caches.get(&cache_dir).and_then(Weak::upgrade) {
        return live;
    }

    let cache = Arc::new(DiskCodeCache::new(cache_dir.clone()));
    // A host that gives every Engine its own directory would otherwise leave one dead
    // entry behind per Engine it destroys.
    caches.retain(|_, cache| cache.strong_count() > 0);
    caches.insert(cache_dir, Arc::downgrade(&cache));
    cache
}

/// The directory a cache lives in, resolved to one name.
///
/// Created before it is resolved, and resolved before it is used as the registry key:
/// two Engines configured with one directory spelled two ways would otherwise get a
/// budget each, which is the defect the registry exists to prevent. A path that
/// cannot be resolved is used as given rather than refused -- the cache is an
/// optimisation, and a Session must still start without one.
fn code_cache_dir(app_cache_dir: &Path) -> PathBuf {
    let dir = app_cache_dir.join("migo_code_cache");
    let _ = fs::create_dir_all(&dir);
    fs::canonicalize(&dir).unwrap_or(dir)
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

#[cfg(test)]
mod tests {
    use super::{CACHES, MAX_CACHE_SIZE, create_code_cache};
    use std::{fs, path::PathBuf, sync::Weak};

    /// One root per test, because the registry is keyed on the directory and these
    /// tests are about what sharing a directory means.
    fn temp_root(label: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("migo_code_cache_{label}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("a writable temp root");
        root
    }

    #[test]
    fn two_sessions_on_one_directory_share_one_budget() {
        // Each Session asks for a cache the way `HostJsRuntime::new` does, with the
        // Engine's one `code_cache_dir`. Before the directory owned the cache, each
        // got a counter of its own, neither saw the other's writes, and the ceiling
        // this module exists to enforce admitted a multiple of itself.
        let root = temp_root("shared_budget");
        let first = create_code_cache(&root);
        let second = create_code_cache(&root);

        const ENTRY: usize = 2 * 1024 * 1024;
        let payload = vec![0xC5; ENTRY];
        let entries = MAX_CACHE_SIZE as usize / ENTRY + 4;
        for index in 0..entries {
            let asking = if index % 2 == 0 { &first } else { &second };
            asking.set(index as u64, &payload);
        }

        // Measured on disk rather than read from the counter: a counter that lied
        // would otherwise satisfy the assertion it is the subject of.
        let resident = first.scan_total_size();
        assert!(
            resident <= MAX_CACHE_SIZE,
            "two Sessions wrote {} MiB into one directory whose ceiling is {} MiB",
            resident / 1024 / 1024,
            MAX_CACHE_SIZE / 1024 / 1024,
        );
        assert!(
            resident > MAX_CACHE_SIZE / 2,
            "eviction released {} MiB, far more than the ceiling asked for",
            (entries as u64 * ENTRY as u64 - resident) / 1024 / 1024,
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_second_session_reads_what_the_first_compiled() {
        // Section 6.5 puts this cache in the shared tier: the bytecode for a source is
        // the same bytes whichever Session compiled it, and both of them load the same
        // engine extension JS. Giving each Session its own directory would satisfy the
        // budget and lose exactly this, which is why the fix was accounting and not
        // partitioning.
        let root = temp_root("shared_entries");
        let first = create_code_cache(&root);
        first.set(0xC0DE, b"bytecode compiled once");

        let second = create_code_cache(&root);
        assert_eq!(
            second.get(0xC0DE).as_deref(),
            Some(&b"bytecode compiled once"[..]),
            "a Session must not recompile what another already compiled"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn two_engines_with_different_directories_do_not_share_one() {
        // The registry is keyed on the directory, not on the process: a host that does
        // give two Engines two roots gets two caches, and one budget each.
        let first_root = temp_root("engine_one");
        let second_root = temp_root("engine_two");
        let first = create_code_cache(&first_root);
        let second = create_code_cache(&second_root);

        first.set(0xFEED, b"one Engine's bytecode");

        assert!(
            second.get(0xFEED).is_none(),
            "a cache reached another Engine's directory"
        );

        let _ = fs::remove_dir_all(&first_root);
        let _ = fs::remove_dir_all(&second_root);
    }

    #[test]
    fn a_directory_reopened_after_its_last_session_counts_what_is_there() {
        // The registry holds `Weak`, so the cache dies with the last Session holding
        // it and the next Session builds another. A rebuilt counter that started at
        // zero would hand that Session the whole ceiling again on top of a directory
        // that is already full.
        let root = temp_root("reopened");
        const ENTRY: u64 = 3 * 1024 * 1024;
        {
            let cache = create_code_cache(&root);
            cache.set(1, &vec![0x5A; ENTRY as usize]);
        }
        assert!(
            CACHES
                .lock()
                .get(&fs::canonicalize(root.join("migo_code_cache")).expect("the cache dir"))
                .is_none_or(|cache| Weak::strong_count(cache) == 0),
            "the last Session's cache outlived it, so this test proves nothing"
        );

        let reopened = create_code_cache(&root);
        assert_eq!(
            *reopened.directory.read(),
            ENTRY,
            "a reopened cache must count the entries already on disk"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
