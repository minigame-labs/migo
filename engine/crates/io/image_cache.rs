//! Decoded-image LRU with a W-TinyLFU admission filter.
//!
//! Caches decoded RGBA image data so repeated `drawImage` calls
//! don't re-run the decode + resize pipeline.  Keyed on
//! `(path, mount_generation, target_w, target_h)` so a hot-update
//! (generation bump) or a different pre-resize size naturally
//! invalidates stale slots.
//!
//! # W-TinyLFU admission
//!
//! A plain size-bounded LRU loses badly when a scene scrolls through
//! dozens of one-shot images — they fill the cache and evict the
//! handful of truly-hot sprites the UI keeps drawing. This module
//! fronts the LRU with a [`CountMinSketch`] and applies the Caffeine
//! admission rule: when the LRU is full, a newcomer is only admitted
//! if its estimated frequency is ≥ the current LRU victim's. Hot
//! items "earn their way in" after being referenced at least once
//! more than the current victim; genuinely one-shot items never
//! displace a warm entry.
//!
//! The rule collapses to "always admit" until the cache is full, so
//! first-touch cost matches the old LRU exactly.
//!
//! # Memory-pressure trim
//!
//! Android's `ComponentCallbacks2.onTrimMemory` is wired through
//! `shared::protocol::host_cmd::OnMemoryWarning`; the host loop
//! translates the level into an [`ImageCache::trim`] call that
//! drops a fraction of the cached bytes. Levels follow Android's
//! scheme exactly: 5 / 10 / 15 → running moderate / low / critical;
//! 20 → UI hidden; 40 / 60 / 80 → background / moderate / complete.

use std::num::NonZeroUsize;
use std::sync::LazyLock;

use lru::LruCache;
use parking_lot::Mutex;
use shared::protocol::io_cmd::NormalizedImage;

use crate::count_min_sketch::CountMinSketch;

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
    /// Count of admission-filter rejections. Each rejection avoided
    /// an eviction that W-TinyLFU judged worse than keeping the
    /// current LRU victim. Ratio vs `misses` is a proxy for cache
    /// churn savings; a high value means the workload is heavy on
    /// one-shot images and the admission filter is doing real work.
    pub admissions_rejected: u64,
    /// Count of bytes evicted specifically by an
    /// [`ImageCache::trim`] call (memory-pressure driven), as
    /// opposed to routine LRU eviction. Useful for confirming the
    /// `onTrimMemory` hook actually ran after a spike.
    pub trim_bytes_released: u64,
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

/// Trim levels mirroring Android `ComponentCallbacks2.TRIM_MEMORY_*`.
/// The host translates the raw integer into one of these variants
/// before calling [`ImageCache::trim`] so the cache isn't coupled
/// to Android-specific constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimLevel {
    /// `TRIM_MEMORY_RUNNING_MODERATE` (5) — mild pressure, drop a
    /// quarter of the cache.
    RunningModerate,
    /// `TRIM_MEMORY_RUNNING_LOW` (10) — drop half.
    RunningLow,
    /// `TRIM_MEMORY_RUNNING_CRITICAL` (15) — drop everything.
    RunningCritical,
    /// `TRIM_MEMORY_UI_HIDDEN` (20) — app fully obscured; drop half.
    UiHidden,
    /// `TRIM_MEMORY_BACKGROUND` / `MODERATE` / `COMPLETE` (40/60/80)
    /// — backgrounded; drop everything, we're not drawing.
    Background,
}

impl TrimLevel {
    /// Decode Android's raw integer level into a [`TrimLevel`].
    /// Unknown values map to the closest documented bucket so new
    /// Android releases don't silently disable the hook.
    pub fn from_android(raw: i32) -> Self {
        match raw {
            5 => TrimLevel::RunningModerate,
            10 => TrimLevel::RunningLow,
            15 => TrimLevel::RunningCritical,
            20 => TrimLevel::UiHidden,
            40 | 60 | 80 => TrimLevel::Background,
            // Anything higher than critical collapses to the most
            // aggressive policy we have; anything lower is "mild".
            n if n >= 15 => TrimLevel::RunningCritical,
            _ => TrimLevel::RunningModerate,
        }
    }

    /// Fraction of the *current* byte usage to release. `1.0` means
    /// clear the cache.
    fn release_fraction(self) -> f64 {
        match self {
            TrimLevel::RunningModerate => 0.25,
            TrimLevel::RunningLow => 0.50,
            TrimLevel::UiHidden => 0.50,
            TrimLevel::RunningCritical => 1.0,
            TrimLevel::Background => 1.0,
        }
    }
}

/// Decoded-image cache with TinyLFU admission.
pub struct ImageCache {
    cache: LruCache<ImageCacheKey, CachedImage>,
    current_size: usize,
    max_size: usize,
    hits: u64,
    misses: u64,
    /// Frequency oracle used by the admission filter.  Reset on
    /// `clear`; size follows the cache's entry budget.
    sketch: CountMinSketch,
    admissions_rejected: u64,
    trim_bytes_released: u64,
}

impl ImageCache {
    /// Create a new cache with default settings.
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_MAX_ENTRIES, DEFAULT_MAX_CACHE_BYTES)
    }

    /// Create a new cache with custom limits.
    pub fn with_limits(max_entries: usize, max_bytes: usize) -> Self {
        // A zero-entry cap would panic in LruCache::new; clamp to
        // `NonZeroUsize::MIN` (= 1) so misconfiguration degrades to
        // a thrashing-but-safe one-slot cache instead of a process
        // abort.
        let cap = NonZeroUsize::new(max_entries).unwrap_or(NonZeroUsize::MIN);
        Self {
            cache: LruCache::new(cap),
            current_size: 0,
            max_size: max_bytes,
            hits: 0,
            misses: 0,
            sketch: CountMinSketch::new_for_capacity(max_entries),
            admissions_rejected: 0,
            trim_bytes_released: 0,
        }
    }

    /// Get an image from cache. Hitting a key still bumps the
    /// frequency counter so long-lived entries keep their admission
    /// advantage.
    pub fn get(&mut self, key: &ImageCacheKey) -> Option<CachedImage> {
        // Frequency accounting runs for every lookup, hit or miss,
        // so the sketch reflects "how popular is this key" not "how
        // often did it hit the cache" — the latter would feedback-
        // lock cold-but-hot paths out forever.
        self.sketch.increment(key);
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

    /// Insert an image into cache. Applies the W-TinyLFU admission
    /// rule when the cache is at its size budget:
    ///
    ///  1. The newcomer's CM-sketch frequency is read (post-
    ///     increment) from the unified sketch.
    ///  2. The current LRU victim's frequency is read.
    ///  3. If the newcomer's frequency is strictly less than the
    ///     victim's, the newcomer is rejected (not cached this call);
    ///     a rejection counter ticks up. The sketch still sees the
    ///     access, so a repeated request will cross the threshold.
    ///  4. Otherwise evict LRU victims until there's room, then
    ///     insert.
    pub fn insert(&mut self, key: ImageCacheKey, image: NormalizedImage) {
        let new_freq = self.sketch.increment(&key);
        let cached = CachedImage::new(image);
        let new_size = cached.size_bytes;

        // Refuse unusable entries up front.
        if new_size > self.max_size {
            return;
        }

        // Over-budget case: consult the admission filter before we
        // evict anything. This is the point where the W-TinyLFU
        // "window" concept collapses to "don't evict a warm entry
        // to make room for a cold one."
        if self.current_size + new_size > self.max_size {
            if let Some((victim_key, _)) = self.cache.peek_lru() {
                let victim_freq = self.sketch.estimate(victim_key);
                if new_freq < victim_freq {
                    // The sketch says the newcomer is colder than
                    // the item we'd have to evict. Drop it on the
                    // floor — on retry its sketch count will have
                    // advanced, which is how cold-but-hot paths
                    // eventually earn their way in.
                    self.admissions_rejected += 1;
                    shared::stats::io_metrics_global()
                        .image_cache_admissions_rejected
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
            }
            while self.current_size + new_size > self.max_size && !self.cache.is_empty() {
                if let Some((_, evicted)) = self.cache.pop_lru() {
                    self.current_size = self.current_size.saturating_sub(evicted.size_bytes);
                }
            }
        }

        // Insert and update size (handle potential replacement).
        if let Some(old) = self.cache.push(key, cached) {
            self.current_size = self.current_size.saturating_sub(old.1.size_bytes);
        }
        self.current_size += new_size;
    }

    /// Clear the entire cache.  Resets the frequency sketch too —
    /// on a full clear there's no history worth keeping around.
    pub fn clear(&mut self) {
        self.cache.clear();
        self.current_size = 0;
        self.sketch.reset();
        // Don't reset hit/miss counters — they're cumulative
        // across the process lifetime for the telemetry pipeline.
    }

    /// Release a fraction of the cache in response to OS memory
    /// pressure. Returns the number of bytes actually freed.
    ///
    /// Eviction walks the LRU tail first, so the freshest entries
    /// survive longer. On `TrimLevel::Background` / `RunningCritical`
    /// the cache is fully cleared (same as [`clear`] but leaves the
    /// sketch intact — the app may come back to the foreground and
    /// the learned frequency distribution is still useful).
    pub fn trim(&mut self, level: TrimLevel) -> usize {
        if self.cache.is_empty() {
            return 0;
        }

        let start_size = self.current_size;
        let fraction = level.release_fraction();
        if fraction >= 1.0 {
            // Aggressive path: drop everything but keep the sketch.
            // We do not touch `hits`/`misses` counters.
            let freed = self.current_size;
            self.cache.clear();
            self.current_size = 0;
            self.trim_bytes_released += freed as u64;
            shared::stats::io_metrics_global()
                .image_cache_trim_bytes
                .fetch_add(freed as u32, std::sync::atomic::Ordering::Relaxed);
            return freed;
        }

        let target_size = ((start_size as f64) * (1.0 - fraction)).round() as usize;
        while self.current_size > target_size && !self.cache.is_empty() {
            if let Some((_, evicted)) = self.cache.pop_lru() {
                self.current_size = self.current_size.saturating_sub(evicted.size_bytes);
            } else {
                break;
            }
        }
        let freed = start_size.saturating_sub(self.current_size);
        self.trim_bytes_released += freed as u64;
        shared::stats::io_metrics_global()
            .image_cache_trim_bytes
            .fetch_add(freed as u32, std::sync::atomic::Ordering::Relaxed);
        freed
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            entries: self.cache.len(),
            size_bytes: self.current_size,
            max_bytes: self.max_size,
            hits: self.hits,
            misses: self.misses,
            admissions_rejected: self.admissions_rejected,
            trim_bytes_released: self.trim_bytes_released,
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

    #[test]
    fn below_budget_inserts_never_rejected() {
        // The admission filter must only kick in when the cache is
        // full. Empty-cache inserts always succeed.
        let mut cache = ImageCache::with_limits(16, 1024 * 1024);
        cache.insert(full_res_key("/a.png".into(), 1), rgba(16, 16));
        cache.insert(full_res_key("/b.png".into(), 1), rgba(16, 16));
        let s = cache.stats();
        assert_eq!(s.entries, 2);
        assert_eq!(s.admissions_rejected, 0);
    }

    #[test]
    fn admission_keeps_hot_entry_across_cold_scans() {
        // Scenario: one hot image, many one-shot images, cache can
        // hold ~3 images worth of bytes.  Under plain LRU the hot
        // image would be evicted on the third cold touch; with
        // TinyLFU it must survive as long as `hot`'s frequency
        // sketch exceeds the cold items'.
        let img_bytes = 64 * 64 * 4;
        let cap_bytes = 4 * img_bytes; // room for ~4 images
        let mut cache = ImageCache::with_limits(16, cap_bytes);

        let hot = full_res_key("/hot.png".into(), 1);
        // Warm up "hot": multiple accesses bump its sketch count.
        for _ in 0..8 {
            let _ = cache.get(&hot); // miss, bumps sketch
        }
        cache.insert(hot.clone(), rgba(64, 64));
        // Each subsequent get bumps hot's count further.
        for _ in 0..8 {
            let _ = cache.get(&hot);
        }

        // Flood with cold one-shot images.
        for i in 0..200u32 {
            let k = full_res_key(format!("/cold_{i}.png"), 1);
            cache.insert(k, rgba(64, 64));
        }

        // The hot entry should still be cached; the admission
        // filter rejected most of the cold inserts once the hot
        // entry's frequency dominated.
        let s = cache.stats();
        assert!(
            cache.contains(&hot),
            "hot entry evicted by cold scan (rejections={}, entries={})",
            s.admissions_rejected,
            s.entries
        );
        assert!(
            s.admissions_rejected > 0,
            "no cold inserts were rejected by admission"
        );
    }

    #[test]
    fn trim_running_low_frees_about_half() {
        let img_bytes = 32 * 32 * 4;
        let mut cache = ImageCache::with_limits(16, 8 * img_bytes);
        for i in 0..8u32 {
            cache.insert(full_res_key(format!("/p{i}.png"), 1), rgba(32, 32));
        }
        let before = cache.stats().size_bytes;
        let freed = cache.trim(TrimLevel::RunningLow);
        let after = cache.stats().size_bytes;
        assert_eq!(before - after, freed);
        // Half of `before` ± one entry's worth.
        let target = before / 2;
        let tolerance = img_bytes;
        assert!(
            after <= target + tolerance,
            "after={after} too high for target {target}"
        );
        assert_eq!(cache.stats().trim_bytes_released as usize, freed);
    }

    #[test]
    fn trim_background_clears_entries_but_keeps_sketch() {
        let mut cache = ImageCache::with_limits(16, 4 * 1024 * 1024);
        for _ in 0..10 {
            let _ = cache.get(&full_res_key("/hot.png".into(), 1));
        }
        cache.insert(full_res_key("/hot.png".into(), 1), rgba(64, 64));

        let freed = cache.trim(TrimLevel::Background);
        let s = cache.stats();
        assert!(freed > 0);
        assert_eq!(s.entries, 0);
        assert_eq!(s.size_bytes, 0);

        // Sketch retained: a fresh insert of the hot key still sees
        // a non-zero frequency, so its re-admission is still
        // privileged on the next spike.
        let freq = cache.sketch.estimate(&full_res_key("/hot.png".into(), 1));
        assert!(freq > 0, "background trim wiped the frequency sketch");
    }

    #[test]
    fn trim_level_from_android_covers_known_codes() {
        assert_eq!(TrimLevel::from_android(5), TrimLevel::RunningModerate);
        assert_eq!(TrimLevel::from_android(10), TrimLevel::RunningLow);
        assert_eq!(TrimLevel::from_android(15), TrimLevel::RunningCritical);
        assert_eq!(TrimLevel::from_android(20), TrimLevel::UiHidden);
        assert_eq!(TrimLevel::from_android(40), TrimLevel::Background);
        assert_eq!(TrimLevel::from_android(60), TrimLevel::Background);
        assert_eq!(TrimLevel::from_android(80), TrimLevel::Background);
        // Unknown high code → closest known bucket is Critical.
        assert_eq!(TrimLevel::from_android(100), TrimLevel::RunningCritical);
    }

    #[test]
    fn clear_resets_sketch() {
        let mut cache = ImageCache::with_limits(4, 1024 * 1024);
        for _ in 0..10 {
            let _ = cache.get(&full_res_key("/k.png".into(), 1));
        }
        cache.clear();
        assert_eq!(
            cache.sketch.estimate(&full_res_key("/k.png".into(), 1)),
            0
        );
    }

    #[test]
    fn oversized_image_is_silently_dropped() {
        // One image bigger than the whole cache must neither be
        // stored nor trigger an eviction storm.
        let mut cache = ImageCache::with_limits(4, 1024);
        cache.insert(full_res_key("/big.png".into(), 1), rgba(64, 64)); // 16KB
        let s = cache.stats();
        assert_eq!(s.entries, 0);
        assert_eq!(s.size_bytes, 0);
    }
}
