//! Audio buffer cache with LRU eviction
//!
//! Caches decoded audio data to avoid repeated downloads and decoding.
//! Uses URL as key and applies LRU eviction when memory limit is exceeded.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use crate::decoder::DecodedAudio;

/// Cache entry with metadata
struct CacheEntry {
    /// Decoded audio data (shared across players)
    audio: Arc<DecodedAudio>,
    /// Size in bytes (samples.len() * 4)
    size_bytes: usize,
    /// Access order (higher = more recent)
    access_order: u64,
}

/// Global audio cache with LRU eviction
pub struct AudioCache {
    /// URL -> CacheEntry
    entries: HashMap<String, CacheEntry>,
    /// access_order -> URL for O(1) LRU eviction
    order_index: BTreeMap<u64, String>,
    /// Current total size in bytes
    current_size: usize,
    /// Maximum cache size in bytes (default: 128MB)
    max_size: usize,
    /// Access counter for LRU ordering
    access_counter: u64,
}

impl AudioCache {
    /// Create new cache with default 128MB limit
    pub fn new() -> Self {
        Self::with_max_size(128 * 1024 * 1024)
    }

    /// Default initial capacity for cache entries.
    /// Most games load a moderate number of audio files.
    const DEFAULT_CACHE_CAPACITY: usize = 16;

    /// Create cache with custom size limit
    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(Self::DEFAULT_CACHE_CAPACITY),
            order_index: BTreeMap::new(),
            current_size: 0,
            max_size,
            access_counter: 0,
        }
    }

    /// Get cached audio by URL
    pub fn get(&mut self, url: &str) -> Option<Arc<DecodedAudio>> {
        if let Some(entry) = self.entries.get_mut(url) {
            // Update access order in both entry and index
            self.order_index.remove(&entry.access_order);
            self.access_counter += 1;
            entry.access_order = self.access_counter;
            self.order_index
                .insert(self.access_counter, url.to_string());
            tracing::trace!("Cache hit: {}", url);
            Some(Arc::clone(&entry.audio))
        } else {
            tracing::trace!("Cache miss: {}", url);
            None
        }
    }

    /// Insert decoded audio into cache
    pub fn insert(&mut self, url: String, audio: DecodedAudio) -> Arc<DecodedAudio> {
        let size_bytes = audio.samples.len() * std::mem::size_of::<f32>();

        // Don't cache if single item exceeds max size
        if size_bytes > self.max_size {
            tracing::debug!(
                "Audio too large to cache: {} bytes (max: {})",
                size_bytes,
                self.max_size
            );
            return Arc::new(audio);
        }

        // Evict entries until we have space
        while self.current_size + size_bytes > self.max_size && !self.entries.is_empty() {
            self.evict_lru();
        }

        self.access_counter += 1;
        let audio = Arc::new(audio);

        // Remove old entry if exists
        if let Some(old) = self.entries.remove(&url) {
            self.current_size -= old.size_bytes;
            self.order_index.remove(&old.access_order);
        }

        self.entries.insert(
            url.clone(),
            CacheEntry {
                audio: Arc::clone(&audio),
                size_bytes,
                access_order: self.access_counter,
            },
        );
        self.order_index.insert(self.access_counter, url.clone());
        self.current_size += size_bytes;

        tracing::debug!(
            "Cached audio: {} ({} bytes, total cache: {} bytes)",
            url,
            size_bytes,
            self.current_size
        );

        audio
    }

    /// Evict least recently used entry (O(1) via BTreeMap)
    fn evict_lru(&mut self) {
        if let Some((&order, _)) = self.order_index.iter().next() {
            if let Some(url) = self.order_index.remove(&order) {
                if let Some(entry) = self.entries.remove(&url) {
                    self.current_size -= entry.size_bytes;
                    tracing::debug!("Evicted from cache: {} ({} bytes)", url, entry.size_bytes);
                }
            }
        }
    }

    /// Remove specific URL from cache
    pub fn remove(&mut self, url: &str) {
        if let Some(entry) = self.entries.remove(url) {
            self.current_size -= entry.size_bytes;
            self.order_index.remove(&entry.access_order);
            tracing::debug!("Removed from cache: {} ({} bytes)", url, entry.size_bytes);
        }
    }

    /// Clear entire cache
    pub fn clear(&mut self) {
        self.entries.clear();
        self.order_index.clear();
        self.current_size = 0;
        self.access_counter = 0;
        tracing::debug!("Cache cleared");
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            entry_count: self.entries.len(),
            current_size: self.current_size,
            max_size: self.max_size,
        }
    }
}

impl Default for AudioCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub entry_count: usize,
    pub current_size: usize,
    pub max_size: usize,
}

/// Thread-safe global cache wrapper
pub struct GlobalAudioCache {
    inner: Mutex<AudioCache>,
}

impl GlobalAudioCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(AudioCache::new()),
        }
    }

    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            inner: Mutex::new(AudioCache::with_max_size(max_size)),
        }
    }

    pub fn get(&self, url: &str) -> Option<Arc<DecodedAudio>> {
        self.inner.lock().ok()?.get(url)
    }

    pub fn insert(&self, url: String, audio: DecodedAudio) -> Arc<DecodedAudio> {
        match self.inner.lock() {
            Ok(mut cache) => cache.insert(url, audio),
            Err(_) => Arc::new(audio),
        }
    }

    pub fn remove(&self, url: &str) {
        if let Ok(mut cache) = self.inner.lock() {
            cache.remove(url);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut cache) = self.inner.lock() {
            cache.clear();
        }
    }

    pub fn stats(&self) -> Option<CacheStats> {
        self.inner.lock().ok().map(|c| c.stats())
    }
}

impl Default for GlobalAudioCache {
    fn default() -> Self {
        Self::new()
    }
}
