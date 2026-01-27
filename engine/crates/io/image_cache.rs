//! LRU cache for decoded images.
//!
//! Caches decoded RGBA image data to avoid re-decoding frequently used images.
//! Uses a size-based eviction policy to control memory usage.

use lru::LruCache;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use shared::protocol::io_cmd::NormalizedImage;
use std::num::NonZeroUsize;

/// Default maximum cache size: 64 MB
const DEFAULT_MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Default maximum number of cached entries
const DEFAULT_MAX_ENTRIES: usize = 256;

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

/// LRU cache for decoded images.
pub struct ImageCache {
    cache: LruCache<String, CachedImage>,
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
    pub fn get(&mut self, path: &str) -> Option<CachedImage> {
        match self.cache.get(path) {
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
    pub fn insert(&mut self, path: String, image: NormalizedImage) {
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
        if let Some(old) = self.cache.push(path, cached) {
            self.current_size = self.current_size.saturating_sub(old.1.size_bytes);
        }
        self.current_size += new_size;
    }

    /// Remove an image from cache.
    pub fn remove(&mut self, path: &str) -> Option<CachedImage> {
        self.cache.pop(path).map(|cached| {
            self.current_size = self.current_size.saturating_sub(cached.size_bytes);
            cached
        })
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

    /// Check if a path is cached.
    pub fn contains(&self, path: &str) -> bool {
        self.cache.contains(path)
    }
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Global image cache instance.
static GLOBAL_CACHE: Lazy<Mutex<ImageCache>> = Lazy::new(|| Mutex::new(ImageCache::new()));

/// Get reference to global cache.
pub fn global_cache() -> parking_lot::MutexGuard<'static, ImageCache> {
    GLOBAL_CACHE.lock()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_image(width: u32, height: u32) -> NormalizedImage {
        let size = (width as usize) * (height as usize) * 4;
        NormalizedImage {
            width,
            height,
            rgba: vec![0u8; size],
        }
    }

    #[test]
    fn test_cache_insert_get() {
        let mut cache = ImageCache::with_limits(10, 1024 * 1024);

        let img = make_test_image(100, 100); // 40KB
        cache.insert("test.png".to_string(), img);

        let retrieved = cache.get("test.png");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().image.width, 100);
    }

    #[test]
    fn test_cache_miss() {
        let mut cache = ImageCache::with_limits(10, 1024 * 1024);
        let retrieved = cache.get("nonexistent.png");
        assert!(retrieved.is_none());
    }

    #[test]
    fn test_cache_eviction() {
        // Cache with 100KB limit
        let mut cache = ImageCache::with_limits(10, 100 * 1024);

        // Insert 3 images of 40KB each (120KB total, exceeds 100KB)
        for i in 0..3 {
            let img = make_test_image(100, 100); // 40KB
            cache.insert(format!("img{}.png", i), img);
        }

        // First image should be evicted
        assert!(cache.get("img0.png").is_none());
        // Last two should exist
        assert!(cache.get("img1.png").is_some());
        assert!(cache.get("img2.png").is_some());
    }

    #[test]
    fn test_cache_stats() {
        let mut cache = ImageCache::with_limits(10, 1024 * 1024);

        let img = make_test_image(100, 100);
        cache.insert("test.png".to_string(), img);

        // One miss
        cache.get("nonexistent.png");
        // One hit
        cache.get("test.png");

        let stats = cache.stats();
        assert_eq!(stats.entries, 1);
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
        assert!((stats.hit_rate() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_cache_clear() {
        let mut cache = ImageCache::with_limits(10, 1024 * 1024);

        let img = make_test_image(100, 100);
        cache.insert("test.png".to_string(), img);

        cache.clear();

        assert!(cache.get("test.png").is_none());
        assert_eq!(cache.stats().entries, 0);
    }
}
