//! LRU cache for decoded images.
//!
//! Caches decoded RGBA image data to avoid re-decoding frequently used images.
//! Uses a size-based eviction policy to control memory usage.
//!
//! Keys are `(path, mount_generation)` tuples so that hot-update / subpackage
//! replacement invalidates stale entries even when the filesystem path is
//! unchanged.

use lru::LruCache;
use parking_lot::Mutex;
use shared::protocol::io_cmd::NormalizedImage;
use std::num::NonZeroUsize;
use std::sync::LazyLock;

/// Default maximum cache size: 64 MB
const DEFAULT_MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Default maximum number of cached entries
const DEFAULT_MAX_ENTRIES: usize = 256;

/// Structured cache key: `(real_path, mount_generation, target_w, target_h)`.
///
/// `target_w = target_h = 0` means "full resolution" — the path most
/// Canvas `drawImage` hits.  Non-zero targets correspond to
/// `createImageBitmap` / `drawImage(…, dw, dh)` paths that request a
/// specific pre-resize, so those get their own cache slots instead of
/// hammering the decode+resize pipeline each frame.
pub type ImageCacheKey = (String, u64, u32, u32);

/// Convenience constructor for the common full-resolution key.
#[inline]
pub fn full_res_key(path: String, generation: u64) -> ImageCacheKey {
    (path, generation, 0, 0)
}

/// Convenience: key for a specific pre-resized variant.  `target_w` or
/// `target_h` == 0 is normalised to 0 to match the "full res" sentinel.
#[inline]
pub fn resized_key(
    path: String,
    generation: u64,
    target_w: u32,
    target_h: u32,
) -> ImageCacheKey {
    (path, generation, target_w, target_h)
}

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

    /// Check whether an image key is currently cached without affecting hit/miss stats.
    pub fn contains(&self, key: &ImageCacheKey) -> bool {
        self.cache.contains(key)
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

#[cfg(test)]
mod tests {
    use super::*;
    use shared::protocol::io_cmd::NormalizedImage;
    use std::sync::Arc;

    fn rgba(width: u32, height: u32) -> NormalizedImage {
        NormalizedImage {
            width,
            height,
            rgba: Arc::new(vec![0u8; (width * height * 4) as usize]),
        }
    }

    #[test]
    fn full_res_and_resized_keys_do_not_collide() {
        // Regression: the same source path at full-res vs a specific
        // pre-resized dimension used to share a cache slot (resizes
        // were never cached at all), forcing the decoder to re-run
        // every frame.  With the 4-tuple key, each variant gets its
        // own slot and survives across draws.
        let mut cache = ImageCache::with_limits(16, 16 * 1024 * 1024);
        let full = full_res_key("/code/sprite.png".into(), 3);
        let r128 = resized_key("/code/sprite.png".into(), 3, 128, 128);
        let r64 = resized_key("/code/sprite.png".into(), 3, 64, 64);

        cache.insert(full.clone(), rgba(256, 256));
        cache.insert(r128.clone(), rgba(128, 128));
        cache.insert(r64.clone(), rgba(64, 64));

        assert_eq!(cache.get(&full).unwrap().image.width, 256);
        assert_eq!(cache.get(&r128).unwrap().image.width, 128);
        assert_eq!(cache.get(&r64).unwrap().image.width, 64);
        let stats = cache.stats();
        assert_eq!(stats.entries, 3);
        assert!(stats.hits >= 3);
    }

    #[test]
    fn generation_bump_evicts_all_sizes() {
        // A mount-generation bump (asset hot-reload) must invalidate
        // every variant of the path, which is naturally handled by
        // keying on generation: gen=9 entries simply never collide
        // with gen=10 lookups.
        let mut cache = ImageCache::with_limits(16, 16 * 1024 * 1024);
        cache.insert(full_res_key("/code/t.png".into(), 9), rgba(32, 32));
        cache.insert(resized_key("/code/t.png".into(), 9, 16, 16), rgba(16, 16));

        assert!(cache
            .get(&full_res_key("/code/t.png".into(), 10))
            .is_none());
        assert!(cache
            .get(&resized_key("/code/t.png".into(), 10, 16, 16))
            .is_none());
    }
}
