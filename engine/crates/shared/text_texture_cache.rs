//! Process-global cache of rasterized text textures.
//!
//! Cocos re-renders the same labels every frame and every shop open
//! ("立即购买", "12000金币", etc.) through the
//! `offscreen Canvas2D → fillText → texImage2D(canvas)` pattern.
//! On each repaint we pay ~4 ms of Skia glyph layout + atlas
//! rasterization + snapshot blit + texImage upload, multiplied by
//! dozens of labels per scene.
//!
//! This module caches the **final GL texture** produced for each
//! distinct `(text, font, size, color, align, baseline, canvas-size)`
//! tuple, keyed across the entire process so subsequent shop opens
//! and sub-views hit the cache.
//!
//! # Design summary
//!
//! - Storage: `LruCache<TextCacheKey, CachedTextEntry>` keyed on
//!   the value-equal `TextCacheKey`, byte- and entry-bounded with
//!   LRU eviction on insert.
//! - Pin / refcount: entries currently in flight (JS-side recognized
//!   a hit and emitted a `TexImage2DFromTextCache` command) are
//!   pinned so the LRU cannot evict them out from under the render
//!   thread before the command executes.  Identical to the
//!   `io::image_cache` pattern.
//! - Eviction returns GL texture ids to the caller (via the
//!   `evicted_textures` Vec on `insert` and via `trim`'s return
//!   value); GL `glDeleteTextures` happens on the render thread,
//!   never inside the cache.
//! - Process-global instance via `LazyLock`.  Access from both the
//!   JS thread (`contains` / `pin` to record a hit) and the render
//!   thread (`insert` / `lookup` / `trim`) under a single
//!   `parking_lot::Mutex`.
//!
//! # Texture ownership invariant
//!
//! A texture id inserted under a `TextCacheKey` is owned by the cache
//! until:
//! - it is removed by LRU eviction during a later `insert` (the
//!   removed id is returned to the caller for deletion), or
//! - the cache is cleared / trimmed (returned ids → caller deletes),
//!   or
//! - the process exits.
//!
//! Pinned entries cannot be evicted; an `insert` whose key is already
//! pinned simply replaces the previous entry under that key (the old
//! id is returned for deletion, the pin transfers to the new entry).

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

use lru::LruCache;
use parking_lot::Mutex;

use crate::protocol::render_cmd::{TextAlign, TextBaseline};

/// Default byte budget.  Text textures are small (typical label
/// ~10–50 KB at 1× DPR), so the byte budget caps faster than the
/// entry count in practice.  Tunable; the on-trim path frees as
/// needed under memory pressure.
const DEFAULT_MAX_BYTES: usize = 128 * 1024 * 1024;

/// Default entry count.  A few thousand labels is well above what a
/// single game session realistically renders.
const DEFAULT_MAX_ENTRIES: usize = 8192;

/// Per-font-family generation counter.  Bumping the generation for a
/// family invalidates all cache entries that referenced the old
/// version, by virtue of those keys carrying the now-stale generation
/// value.  Stale entries naturally age out via LRU; bumping the
/// counter does not eagerly evict.
///
/// Render thread calls [`bump_font_generation`] each time `LoadFont`
/// registers / replaces a typeface for a family name.
static FONT_GENERATION_BASE: AtomicU64 = AtomicU64::new(1);

/// Returns the next monotonic generation value.  Called by the render
/// thread on font load / replace; cache keys captured by JS embed the
/// **current** generation for the family at fillText time, so an entry
/// keyed under generation `G` becomes unreachable once the family
/// rolls forward to `G+1`.
#[inline]
pub fn bump_font_generation() -> u64 {
    FONT_GENERATION_BASE.fetch_add(1, Ordering::Relaxed) + 1
}

/// Read the current generation token (without bumping it).  JS uses
/// this when constructing a `TextCacheKey` to capture the family's
/// current version.
#[inline]
pub fn current_font_generation() -> u64 {
    FONT_GENERATION_BASE.load(Ordering::Relaxed)
}

/// Cache key.  `Hash` + `Eq` derive over all fields; `f32` is mapped
/// to its bit pattern (`to_bits`) for hashing so two `NaN` values
/// would compare equal (acceptable — cocos never passes NaN sizes).
///
/// Field set is the **minimum** required for correctness: dropping
/// any one of these means two different renderings collapse to one
/// cache slot.  Adding fields (e.g. stroke color, shadow) would
/// reduce hit rate, so kept tight until the whitelist expands to
/// cover those.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextCacheKey {
    /// The text string passed to `fillText`.
    pub text: String,
    /// The raw `font` CSS string the caller set on the Canvas2D
    /// context.  We intentionally key on the unresolved request
    /// string rather than the resolved family because cocos always
    /// passes the same request string for the same intent;
    /// resolution happens on the render thread and would force a
    /// sync round-trip if we wanted the resolved name on the JS
    /// side.  Duplicates from "Arial, sans-serif" vs "Arial" are
    /// acceptable — they're rare and self-limiting.
    pub font_request: String,
    /// `f32::to_bits()` of the font size in CSS pixels.
    pub font_size_bits: u32,
    /// Font weight; 400 for regular, 700 for bold.
    pub font_weight: u16,
    pub italic: bool,
    /// RGBA u32 (premultiplied alpha not applied here — Skia handles
    /// alpha in its paint pipeline).
    pub fill_color: u32,
    pub text_align: TextAlign,
    pub text_baseline: TextBaseline,
    /// Canvas dimensions in device pixels.  The produced texture has
    /// the same dimensions; two canvases of different sizes calling
    /// fillText with otherwise identical state produce different
    /// textures (different padding around the glyph run).
    pub canvas_w: u32,
    pub canvas_h: u32,
    /// Font generation snapshot (see [`current_font_generation`]).
    /// Rolls forward on font reload; old-generation entries become
    /// unreachable.
    pub font_generation: u64,
}

/// Resident cache entry.  `texture_id` is the GL name owned by the
/// cache (not by any `ImageStore` / `image_registry`).  Render thread
/// is the only mutator; JS-side callers read `width`/`height` for
/// sanity checks but never touch GL state.
#[derive(Debug, Clone)]
pub struct CachedTextEntry {
    pub texture_id: u32,
    pub width: u32,
    pub height: u32,
    /// Resident byte cost for budget accounting (RGBA8 → 4 × w × h).
    pub size_bytes: usize,
}

/// Trim levels from `ComponentCallbacks2.onTrimMemory`, mirroring
/// `io::image_cache::TrimLevel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimLevel {
    RunningModerate,
    RunningLow,
    RunningCritical,
    UiHidden,
    Background,
}

impl TrimLevel {
    pub fn from_android(raw: i32) -> Self {
        match raw {
            5 => TrimLevel::RunningModerate,
            10 => TrimLevel::RunningLow,
            15 => TrimLevel::RunningCritical,
            20 => TrimLevel::UiHidden,
            40 | 60 | 80 => TrimLevel::Background,
            n if n >= 15 => TrimLevel::RunningCritical,
            _ => TrimLevel::RunningModerate,
        }
    }

    fn release_fraction(self) -> f64 {
        match self {
            TrimLevel::RunningModerate => 0.25,
            TrimLevel::RunningLow | TrimLevel::UiHidden => 0.50,
            TrimLevel::RunningCritical | TrimLevel::Background => 1.0,
        }
    }
}

/// Snapshot of cache statistics for diagnostics.
#[derive(Debug, Clone, Copy, Default)]
pub struct TextCacheStats {
    pub entries: usize,
    pub size_bytes: usize,
    pub max_bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub trim_bytes_released: u64,
}

impl TextCacheStats {
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

pub struct TextTextureCache {
    cache: LruCache<TextCacheKey, CachedTextEntry>,
    pins: HashMap<TextCacheKey, u32>,
    current_size_bytes: usize,
    max_size_bytes: usize,
    hits: u64,
    misses: u64,
    trim_bytes_released: u64,
}

impl Default for TextTextureCache {
    fn default() -> Self {
        Self::with_limits(DEFAULT_MAX_ENTRIES, DEFAULT_MAX_BYTES)
    }
}

impl TextTextureCache {
    pub fn with_limits(max_entries: usize, max_bytes: usize) -> Self {
        let cap = NonZeroUsize::new(max_entries).unwrap_or(NonZeroUsize::MIN);
        Self {
            cache: LruCache::new(cap),
            pins: HashMap::new(),
            current_size_bytes: 0,
            max_size_bytes: max_bytes,
            hits: 0,
            misses: 0,
            trim_bytes_released: 0,
        }
    }

    /// Lookup without mutating LRU order — used as a cheap "would
    /// this hit" probe from JS without committing to use it yet.
    /// Returns a clone of the entry so the caller can release the
    /// lock immediately.
    pub fn peek(&self, key: &TextCacheKey) -> Option<CachedTextEntry> {
        self.cache.peek(key).cloned()
    }

    /// Lookup AND promote to MRU.  Bumps the hit counter.  Render
    /// thread calls this when consuming a hit.
    pub fn get(&mut self, key: &TextCacheKey) -> Option<CachedTextEntry> {
        if let Some(entry) = self.cache.get(key) {
            self.hits += 1;
            Some(entry.clone())
        } else {
            self.misses += 1;
            None
        }
    }

    /// Increment the pin count for `key`.  Pinned entries are exempt
    /// from LRU eviction in `insert` / `trim` / `clear`.  Safe to
    /// call before the entry has been inserted — the pin is recorded
    /// and applies to whichever entry next occupies that key.
    pub fn pin(&mut self, key: &TextCacheKey) {
        *self.pins.entry(key.clone()).or_insert(0) += 1;
    }

    /// Decrement the pin count; remove the pin record at zero.
    /// Tolerant of extra unpins (no-op rather than panic).
    pub fn unpin(&mut self, key: &TextCacheKey) {
        if let Some(count) = self.pins.get_mut(key) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.pins.remove(key);
            }
        }
    }

    pub fn pin_count(&self, key: &TextCacheKey) -> u32 {
        self.pins.get(key).copied().unwrap_or(0)
    }

    /// Insert a freshly-rendered entry under `key`.  Returns the list
    /// of GL texture ids the caller must delete on the render thread
    /// — these are LRU victims and any prior occupant of `key`.
    /// Refuses to insert oversized entries (silently drops; texture
    /// id is returned for deletion).
    #[must_use = "texture ids in the returned Vec must be deleted on the render thread"]
    pub fn insert(&mut self, key: TextCacheKey, entry: CachedTextEntry) -> Vec<u32> {
        let mut evicted: Vec<u32> = Vec::new();

        // Oversized inserts: never store, hand the texture back for
        // immediate deletion.
        if entry.size_bytes > self.max_size_bytes {
            evicted.push(entry.texture_id);
            return evicted;
        }

        // Evict LRU tail until we have room (or only pinned entries
        // remain — in which case the cache is allowed to sit
        // over-budget, matching `io::image_cache`'s contract).
        while self.current_size_bytes + entry.size_bytes > self.max_size_bytes {
            if let Some(victim_key) = self
                .cache
                .iter()
                .rev()
                .find(|(k, _)| !self.pins.contains_key(*k))
                .map(|(k, _)| k.clone())
            {
                if let Some(victim) = self.cache.pop(&victim_key) {
                    self.current_size_bytes =
                        self.current_size_bytes.saturating_sub(victim.size_bytes);
                    evicted.push(victim.texture_id);
                }
            } else {
                break;
            }
        }

        // Insert.  `LruCache::push` evicts the absolute tail if at
        // the entry-count cap; that eviction is independent of our
        // byte-budget logic and may displace a pinned entry, which
        // we must avoid.  Manually evict an unpinned tail first if
        // we're at the entry cap.
        if self.cache.len() >= self.cache.cap().get() && !self.cache.contains(&key) {
            if let Some(victim_key) = self
                .cache
                .iter()
                .rev()
                .find(|(k, _)| !self.pins.contains_key(*k))
                .map(|(k, _)| k.clone())
            {
                if let Some(victim) = self.cache.pop(&victim_key) {
                    self.current_size_bytes =
                        self.current_size_bytes.saturating_sub(victim.size_bytes);
                    evicted.push(victim.texture_id);
                }
            }
            // If every entry is pinned and we're at the cap, allow
            // the LRU's own eviction in `push` below to fire; it
            // may displace a pinned entry but the over-pin
            // condition is itself indicating misuse.  In normal
            // operation pins are short-lived (one in-flight
            // command), so the at-cap-all-pinned case is
            // pathological.
        }

        let new_size = entry.size_bytes;
        if let Some((_, old)) = self.cache.push(key, entry) {
            self.current_size_bytes = self.current_size_bytes.saturating_sub(old.size_bytes);
            evicted.push(old.texture_id);
        }
        self.current_size_bytes += new_size;

        evicted
    }

    /// Memory-pressure trim.  Returns the texture ids the caller must
    /// delete on the render thread.  Pinned entries are preserved
    /// (same contract as `io::image_cache::trim`).
    #[must_use]
    pub fn trim(&mut self, level: TrimLevel) -> Vec<u32> {
        let mut evicted: Vec<u32> = Vec::new();
        if self.cache.is_empty() {
            return evicted;
        }

        let start_size = self.current_size_bytes;
        let fraction = level.release_fraction();

        if fraction >= 1.0 {
            // Drop all unpinned entries.
            let drop_keys: Vec<TextCacheKey> = self
                .cache
                .iter()
                .filter(|(k, _)| !self.pins.contains_key(*k))
                .map(|(k, _)| k.clone())
                .collect();
            for k in drop_keys {
                if let Some(victim) = self.cache.pop(&k) {
                    self.current_size_bytes =
                        self.current_size_bytes.saturating_sub(victim.size_bytes);
                    evicted.push(victim.texture_id);
                }
            }
        } else {
            let target = ((start_size as f64) * (1.0 - fraction)).round() as usize;
            while self.current_size_bytes > target {
                let victim_key = self
                    .cache
                    .iter()
                    .rev()
                    .find(|(k, _)| !self.pins.contains_key(*k))
                    .map(|(k, _)| k.clone());
                let Some(k) = victim_key else { break };
                if let Some(victim) = self.cache.pop(&k) {
                    self.current_size_bytes =
                        self.current_size_bytes.saturating_sub(victim.size_bytes);
                    evicted.push(victim.texture_id);
                } else {
                    break;
                }
            }
        }

        let freed = start_size.saturating_sub(self.current_size_bytes);
        self.trim_bytes_released = self.trim_bytes_released.saturating_add(freed as u64);
        evicted
    }

    pub fn stats(&self) -> TextCacheStats {
        TextCacheStats {
            entries: self.cache.len(),
            size_bytes: self.current_size_bytes,
            max_bytes: self.max_size_bytes,
            hits: self.hits,
            misses: self.misses,
            trim_bytes_released: self.trim_bytes_released,
        }
    }
}

/// Process-global cache instance.
static GLOBAL_CACHE: LazyLock<Mutex<TextTextureCache>> =
    LazyLock::new(|| Mutex::new(TextTextureCache::default()));

/// Acquire a guard over the global cache.  Both JS and render thread
/// go through this entry point.
pub fn global_cache() -> parking_lot::MutexGuard<'static, TextTextureCache> {
    GLOBAL_CACHE.lock()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(text: &str, color: u32) -> TextCacheKey {
        TextCacheKey {
            text: text.into(),
            font_request: "Arial 14px".into(),
            font_size_bits: 14.0f32.to_bits(),
            font_weight: 400,
            italic: false,
            fill_color: color,
            text_align: TextAlign::Start,
            text_baseline: TextBaseline::Alphabetic,
            canvas_w: 128,
            canvas_h: 32,
            font_generation: 1,
        }
    }

    fn entry(id: u32, bytes: usize) -> CachedTextEntry {
        CachedTextEntry {
            texture_id: id,
            width: 128,
            height: 32,
            size_bytes: bytes,
        }
    }

    #[test]
    fn insert_then_get_roundtrip() {
        let mut c = TextTextureCache::with_limits(16, 1024 * 1024);
        let key = k("hi", 0xffffffff);
        assert!(c.insert(key.clone(), entry(1, 1000)).is_empty());
        let got = c.get(&key).expect("hit");
        assert_eq!(got.texture_id, 1);
        let s = c.stats();
        assert_eq!(s.entries, 1);
        assert_eq!(s.size_bytes, 1000);
        assert_eq!(s.hits, 1);
    }

    #[test]
    fn lru_evicts_oldest_unpinned() {
        let mut c = TextTextureCache::with_limits(16, 2_000);
        let _ = c.insert(k("a", 1), entry(1, 1000));
        let _ = c.insert(k("b", 2), entry(2, 1000));
        let evicted = c.insert(k("c", 3), entry(3, 1000));
        assert_eq!(evicted, vec![1]); // "a" was LRU tail
        assert!(c.get(&k("a", 1)).is_none());
        assert!(c.get(&k("b", 2)).is_some());
        assert!(c.get(&k("c", 3)).is_some());
    }

    #[test]
    fn pinned_entry_survives_pressure() {
        let mut c = TextTextureCache::with_limits(16, 2_000);
        let pinned = k("p", 1);
        let _ = c.insert(pinned.clone(), entry(1, 1000));
        c.pin(&pinned);
        let _ = c.insert(k("b", 2), entry(2, 1000));
        // Inserting "c" would normally evict the LRU tail ("p"); the
        // pin protects it, so the cache goes over budget instead.
        let evicted = c.insert(k("c", 3), entry(3, 1000));
        // No eviction occurred (over-budget allowed).
        assert!(evicted.is_empty());
        assert!(c.peek(&pinned).is_some());
    }

    #[test]
    fn oversized_entry_dropped_immediately() {
        let mut c = TextTextureCache::with_limits(16, 1024);
        let evicted = c.insert(k("big", 0), entry(99, 4096));
        assert_eq!(evicted, vec![99]);
        assert_eq!(c.stats().entries, 0);
    }

    #[test]
    fn unpin_idempotent_at_zero() {
        let mut c = TextTextureCache::with_limits(4, 1024);
        let key = k("a", 0);
        c.unpin(&key); // no-op
        c.pin(&key);
        c.unpin(&key);
        c.unpin(&key); // tolerated
        assert_eq!(c.pin_count(&key), 0);
    }

    #[test]
    fn trim_running_low_releases_about_half() {
        let mut c = TextTextureCache::with_limits(16, 8 * 1000);
        for i in 0..8u32 {
            let _ = c.insert(k(&format!("k{i}"), i), entry(100 + i, 1000));
        }
        let before = c.stats().size_bytes;
        let evicted = c.trim(TrimLevel::RunningLow);
        let after = c.stats().size_bytes;
        assert!(!evicted.is_empty());
        assert_eq!(before - after, (evicted.len() * 1000));
        assert!(after <= before / 2 + 1000);
    }

    #[test]
    fn trim_background_keeps_pinned() {
        let mut c = TextTextureCache::with_limits(8, 4 * 1000);
        let pinned = k("live", 0);
        let _ = c.insert(pinned.clone(), entry(1, 1000));
        c.pin(&pinned);
        let _ = c.insert(k("idle", 0), entry(2, 1000));
        let evicted = c.trim(TrimLevel::Background);
        assert_eq!(evicted, vec![2]);
        assert!(c.peek(&pinned).is_some());
    }

    #[test]
    fn font_generation_advances() {
        let g0 = current_font_generation();
        let g1 = bump_font_generation();
        assert!(g1 > g0);
        assert_eq!(current_font_generation(), g1);
    }
}
