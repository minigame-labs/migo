//! LRU cache for decoded images.
//!
//! Caches decoded RGBA image data to avoid re-decoding frequently used images.
//! Uses a size-based eviction policy to control memory usage.
//!
//! Keys are `(path, mount_generation)` tuples so that hot-update / subpackage
//! replacement invalidates stale entries even when the filesystem path is
//! unchanged.

use lru::LruCache;
use std::sync::LazyLock;
use parking_lot::Mutex;
use shared::protocol::io_cmd::NormalizedImage;
use std::num::NonZeroUsize;

/// Default maximum cache size: 64 MB
const DEFAULT_MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Default maximum number of cached entries
const DEFAULT_MAX_ENTRIES: usize = 256;

/// Structured cache key: (real_path, mount_generation).
///
/// Using a tuple avoids delimiter collision that string concatenation
/// would introduce (`:` is a legal path character on Linux).
pub type ImageCacheKey = (String, u64);

/// Cached image entry with reference counting.
#[derive(Clone)]
pub struct CachedImage {
    pub image: NormalizedImage,
    size_bytes: usize,
}

impl CachedImage {
    fn new(image: NormalizedImage) -> Self {
        let size_bytes = image.rgba.len();
        Self { image, size_bytes }
    }
}

/// Cache statistics for monitoring.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub entries: usize,
    pub size_bytes: usize,
    pub max_bytes: usize,
    pub hits: u64,
    pub misses: u64,
}

impl CacheStats {
    /// Calculate hit rate (0.0 - 1.0).
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// LRU cache for decoded images, keyed by `(path, mount_generation)`.
pub struct ImageCache {
    cache: LruCache<ImageCacheKey, CachedImage>,
    current_size: usize,
    max_size: usize,
    hits: u64,
    misses: u64,
}

impl ImageCache {
    /// Create a new cache with default settings.
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_MAX_ENTRIES, DEFAULT_MAX_CACHE_BYTES)
    }

    /// Create a new cache with custom limits.
    pub fn with_limits(max_entries: usize, max_bytes: usize) -> Self {
        let cap = NonZeroUsize::new(max_entries).unwrap_or(NonZeroUsize::new(1).unwrap());
        Self {
            cache: LruCache::new(cap),
            current_size: 0,
            max_size: max_bytes,
            hits: 0,
            misses: 0,
        }
    }

    /// Get an image from cache.
    pub fn get(&mut self, key: &ImageCacheKey) -> Option<CachedImage> {
        match self.cache.get(key) {
            Some(cached) => {
                self.hits += 1;
                Some(cached.clone())
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Insert an image into cache.
    pub fn insert(&mut self, key: ImageCacheKey, image: NormalizedImage) {
        let cached = CachedImage::new(image);
        let new_size = cached.size_bytes;

        // Evict entries until we have room
        while self.current_size + new_size > self.max_size && !self.cache.is_empty() {
            if let Some((_, evicted)) = self.cache.pop_lru() {
                self.current_size = self.current_size.saturating_sub(evicted.size_bytes);
            }
        }

        // If the single image is larger than max_size, don't cache it
        if new_size > self.max_size {
            return;
        }

        // Insert and update size (handle potential replacement)
        if let Some(old) = self.cache.push(key, cached) {
            self.current_size = self.current_size.saturating_sub(old.1.size_bytes);
        }
        self.current_size += new_size;
    }

    /// Clear the entire cache.
    pub fn clear(&mut self) {
        self.cache.clear();
        self.current_size = 0;
        // Don't reset hit/miss counters
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            entries: self.cache.len(),
            size_bytes: self.current_size,
            max_bytes: self.max_size,
            hits: self.hits,
            misses: self.misses,
        }
    }
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Global image cache instance.
static GLOBAL_CACHE: LazyLock<Mutex<ImageCache>> = LazyLock::new(|| Mutex::new(ImageCache::new()));

/// Get reference to global cache.
pub fn global_cache() -> parking_lot::MutexGuard<'static, ImageCache> {
    GLOBAL_CACHE.lock()
}
