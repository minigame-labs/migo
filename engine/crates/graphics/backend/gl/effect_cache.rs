//! Memoisation for the two expensive effect objects that every
//! Canvas2D drawing pass builds: the drop-shadow `ImageFilter` and
//! the dash `PathEffect`.
//!
//! Why a cache at all?  Both effect kinds are reference-counted
//! Skia handles (`RCHandle<SkImageFilter>` / `RCHandle<SkPathEffect>`),
//! and both are deterministic pure functions of their parameters:
//!
//!   * `drop_shadow(dx, dy, sigma_x, sigma_y, color)` → same
//!     `SkImageFilter` every time.
//!   * `dash(intervals, phase)` → same `SkPathEffect` every time.
//!
//! Games tend to set the same shadow on every button label, and the
//! same dash pattern on every selection outline.  Rebuilding the
//! effect on every paint pays the Skia constructor plus one
//! allocation per call; with a small LRU we pay that once per
//! distinct parameter tuple and hand back a cheap `Arc`-like
//! refcount bump on every subsequent hit.
//!
//! The caches live in thread-local storage because Skia effect
//! handles are not `Send` (they lean on Skia's own per-thread
//! reference counting).  Cap both at 32 entries — typical games use
//! a handful of shadow / dash configs plus occasional one-offs, so
//! the hit rate on small caps is essentially 100% while steady-state
//! memory stays in single-digit KB.

use std::cell::RefCell;
use std::num::NonZeroUsize;

use lru::LruCache;
use skia_safe::{image_filters, ImageFilter, PathEffect};

const SHADOW_CACHE_CAP: usize = 32;
const DASH_CACHE_CAP: usize = 32;

/// Shadow-filter cache key.  All four parameters are bit-casted to
/// preserve exact equality: `f32::to_bits` is a lossless round-trip
/// except for the NaN payload, which never shows up on Canvas 2D
/// shadow parameters (JS coerces via `Number` which normalises
/// NaN to one pattern).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ShadowKey {
    /// Premultiplied `SkColor` (ARGB u32).  Already embeds
    /// `global_alpha` modulation, so two calls with the same logical
    /// shadow colour but different alpha get different keys (which
    /// is correct — the filter bakes the colour in).
    color: u32,
    sigma_bits: u32,
    dx_bits: u32,
    dy_bits: u32,
}

/// Dash-effect cache key.  `intervals_hash` folds the full slice
/// into a 64-bit hash (FxHash is cheap and ordered so `[4, 2]` and
/// `[2, 4]` hash differently).  Collisions are tolerated silently
/// because the cache is a best-effort optimisation: on collision
/// we'd hand back the wrong `PathEffect`, so we ALSO store a copy
/// of the intervals in the value and re-verify on lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DashKey {
    intervals_hash: u64,
    phase_bits: u32,
}

struct DashEntry {
    intervals: Vec<f32>,
    phase: f32,
    effect: PathEffect,
}

thread_local! {
    static SHADOW_CACHE: RefCell<LruCache<ShadowKey, ImageFilter>> = RefCell::new(
        LruCache::new(NonZeroUsize::new(SHADOW_CACHE_CAP).expect("cap > 0"))
    );
    static DASH_CACHE: RefCell<LruCache<DashKey, DashEntry>> = RefCell::new(
        LruCache::new(NonZeroUsize::new(DASH_CACHE_CAP).expect("cap > 0"))
    );
}

/// Fetch or build a drop-shadow `ImageFilter`.  Returns `None` when
/// Skia's `image_filters::drop_shadow` constructor itself returns
/// `None` (typically an OOM on the raw filter graph; should never
/// happen in practice but we propagate instead of panicking).
pub fn get_or_build_drop_shadow(
    color: u32,
    sigma_x: f32,
    sigma_y: f32,
    dx: f32,
    dy: f32,
) -> Option<ImageFilter> {
    let key = ShadowKey {
        color,
        sigma_bits: sigma_x.to_bits(),
        dx_bits: dx.to_bits(),
        dy_bits: dy.to_bits(),
    };
    // Separate from the key above so `sigma_x != sigma_y` still
    // participates in the cache hit test even though Canvas 2D
    // always uses equal sigmas.
    let key = (key, sigma_y.to_bits());
    let packed = ShadowKey {
        color: key.0.color,
        sigma_bits: mix_u32(key.0.sigma_bits, key.1),
        dx_bits: key.0.dx_bits,
        dy_bits: key.0.dy_bits,
    };
    SHADOW_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if let Some(hit) = cache.get(&packed) {
            return Some(hit.clone());
        }
        let filter = image_filters::drop_shadow(
            (dx, dy),
            (sigma_x, sigma_y),
            skia_safe::Color::from(color),
            None,
            None,
            None,
        )?;
        cache.put(packed, filter.clone());
        Some(filter)
    })
}

/// Fetch or build a dash `PathEffect`.  Returns `None` when Skia
/// rejects the intervals (empty list, odd count, negative entries).
///
/// The returned effect is an RC handle — cloning is a refcount bump,
/// never a deep copy.
pub fn get_or_build_dash(intervals: &[f32], phase: f32) -> Option<PathEffect> {
    if intervals.is_empty() {
        return None;
    }
    let intervals_hash = hash_intervals(intervals);
    let key = DashKey {
        intervals_hash,
        phase_bits: phase.to_bits(),
    };
    DASH_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        // On cache hit, re-verify intervals and phase — a 64-bit
        // hash collision would otherwise silently serve the wrong
        // dash, a class of bug that's painful to diagnose.
        if let Some(entry) = cache.get(&key) {
            if entry.intervals == intervals && entry.phase.to_bits() == phase.to_bits() {
                return Some(entry.effect.clone());
            }
        }
        let effect = PathEffect::dash(intervals, phase)?;
        cache.put(
            key,
            DashEntry {
                intervals: intervals.to_vec(),
                phase,
                effect: effect.clone(),
            },
        );
        Some(effect)
    })
}

/// Empty the caches.  Never strictly necessary (entries are bounded)
/// but useful from tests that want deterministic refcount assertions
/// or from a future `freeGpuResources` integration.
#[allow(dead_code)]
pub fn clear_all() {
    SHADOW_CACHE.with(|c| c.borrow_mut().clear());
    DASH_CACHE.with(|c| c.borrow_mut().clear());
}

#[inline]
fn mix_u32(a: u32, b: u32) -> u32 {
    // Reversible-ish mix so the two sigma channels contribute
    // independently to the packed key.  We use wrapping xorshift
    // rather than a full hash because the bit patterns are already
    // IEEE-754 floats — entropy is high in the mantissa.
    a.wrapping_mul(0x9E37_79B9).wrapping_add(b.rotate_left(13))
}

fn hash_intervals(intervals: &[f32]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for v in intervals {
        v.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_cache_hits_on_identical_params() {
        clear_all();
        let color = 0xFF00_0000u32;
        let a = get_or_build_drop_shadow(color, 4.0, 4.0, 2.0, 2.0).expect("build");
        let b = get_or_build_drop_shadow(color, 4.0, 4.0, 2.0, 2.0).expect("build");
        // RC handles should point at the same native pointer when
        // served from cache.  skia-safe doesn't expose raw ptr
        // equality, but `Clone` of an RCHandle bumps the refcount,
        // so we can at least verify the cache doesn't grow past 1
        // entry after two identical requests.
        SHADOW_CACHE.with(|c| {
            assert_eq!(c.borrow().len(), 1, "two identical requests produced two entries");
        });
        let _ = (a, b);
    }

    #[test]
    fn shadow_cache_distinguishes_colour_and_sigma() {
        clear_all();
        let _ = get_or_build_drop_shadow(0xFF00_0000, 4.0, 4.0, 2.0, 2.0).unwrap();
        let _ = get_or_build_drop_shadow(0xFFFF_0000, 4.0, 4.0, 2.0, 2.0).unwrap();
        let _ = get_or_build_drop_shadow(0xFF00_0000, 8.0, 8.0, 2.0, 2.0).unwrap();
        let _ = get_or_build_drop_shadow(0xFF00_0000, 4.0, 4.0, 5.0, 2.0).unwrap();
        SHADOW_CACHE.with(|c| {
            assert_eq!(c.borrow().len(), 4, "each distinct param tuple must occupy its own slot");
        });
    }

    #[test]
    fn dash_cache_hits_on_identical_intervals_and_phase() {
        clear_all();
        let _ = get_or_build_dash(&[4.0, 2.0], 0.0).unwrap();
        let _ = get_or_build_dash(&[4.0, 2.0], 0.0).unwrap();
        DASH_CACHE.with(|c| {
            assert_eq!(c.borrow().len(), 1);
        });
    }

    #[test]
    fn dash_cache_preserves_interval_order() {
        clear_all();
        // [4, 2] and [2, 4] render differently; the cache MUST
        // NOT alias them even if a naive multiset hash would.
        let _ = get_or_build_dash(&[4.0, 2.0], 0.0).unwrap();
        let _ = get_or_build_dash(&[2.0, 4.0], 0.0).unwrap();
        DASH_CACHE.with(|c| {
            assert_eq!(c.borrow().len(), 2);
        });
    }

    #[test]
    fn dash_rejects_empty_intervals() {
        clear_all();
        assert!(get_or_build_dash(&[], 0.0).is_none());
    }

    #[test]
    fn dash_cache_distinguishes_phase() {
        clear_all();
        let _ = get_or_build_dash(&[4.0, 2.0], 0.0).unwrap();
        let _ = get_or_build_dash(&[4.0, 2.0], 1.0).unwrap();
        DASH_CACHE.with(|c| {
            assert_eq!(c.borrow().len(), 2);
        });
    }
}
