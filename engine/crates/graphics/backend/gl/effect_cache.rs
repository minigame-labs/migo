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
use skia_safe::{image_filters, gradient_shader, ImageFilter, PathEffect, Point, Shader, TileMode};

use shared::protocol::render_cmd::GradientStop;

const SHADOW_CACHE_CAP: usize = 32;
const DASH_CACHE_CAP: usize = 32;
const GRADIENT_CACHE_CAP: usize = 64;

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

/// Gradient-shader cache key.  Combines the geometric parameters
/// (packed as f32 bit patterns) with an identity-level reference
/// to the stop list (`Arc::as_ptr`) and the global alpha.
///
/// Using the `Arc` pointer address means two `ctx.createLinearGradient`
/// calls that share the same stop vector hit the cache on the second
/// use -- the JS façade always clones the `Arc<Vec<GradientStop>>`
/// inside `StyleKind`, so repeatedly assigning the same gradient
/// object to `fillStyle` produces pointer-equal keys.  Distinct
/// gradient objects with identical stops pay one build then hit.
///
/// Hash collisions on `stops_addr` (Arc drop + new alloc at same
/// address) are defended against by re-verifying content + `kind` +
/// `geom_bits` on cache hit -- same pattern as the dash cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GradientKey {
    kind: u8, // 0 = linear, 1 = radial, 2 = conic
    /// Up to 7 f32 parameters as bit patterns; unused slots are 0.
    /// Linear: [x0, y0, x1, y1, 0, 0, 0]
    /// Radial: [x0, y0, r0, x1, y1, r1, 0]
    /// Conic:  [cx, cy, start_angle, 0, 0, 0, 0]
    geom_bits: [u32; 7],
    alpha_bits: u32,
    stops_addr: usize,
    stops_len: u32,
}

struct GradientEntry {
    /// Snapshot of the stops list the shader was built from.  Used
    /// to re-verify on cache hit against the risk that two different
    /// Arc allocations reuse the same heap address.
    stops_snapshot: Vec<GradientStop>,
    shader: Shader,
}

thread_local! {
    static SHADOW_CACHE: RefCell<LruCache<ShadowKey, ImageFilter>> = RefCell::new(
        LruCache::new(NonZeroUsize::new(SHADOW_CACHE_CAP).expect("cap > 0"))
    );
    static DASH_CACHE: RefCell<LruCache<DashKey, DashEntry>> = RefCell::new(
        LruCache::new(NonZeroUsize::new(DASH_CACHE_CAP).expect("cap > 0"))
    );
    static GRADIENT_CACHE: RefCell<LruCache<GradientKey, GradientEntry>> = RefCell::new(
        LruCache::new(NonZeroUsize::new(GRADIENT_CACHE_CAP).expect("cap > 0"))
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
            crate::render_diagnostics::hit_shadow_filter_cache();
            return Some(hit.clone());
        }
        crate::render_diagnostics::miss_shadow_filter_cache();
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
                crate::render_diagnostics::hit_dash_effect_cache();
                return Some(entry.effect.clone());
            }
            // Hash collision with different payload — fall through
            // to the miss path and overwrite.  Rare; counted as a
            // miss for honest hit-rate accounting.
        }
        crate::render_diagnostics::miss_dash_effect_cache();
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

/// Fetch or build a linear-gradient `Shader`.  Caches on
/// (gradient kind, geometric parameters, stop-list identity, global
/// alpha).  See [`GradientKey`] for the cache-hit re-verification
/// strategy on `Arc::as_ptr` collisions.
pub fn get_or_build_linear_gradient(
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    stops: &std::sync::Arc<Vec<GradientStop>>,
    global_alpha: f32,
) -> Option<Shader> {
    let key = GradientKey {
        kind: 0,
        geom_bits: [
            x0.to_bits(),
            y0.to_bits(),
            x1.to_bits(),
            y1.to_bits(),
            0, 0, 0,
        ],
        alpha_bits: global_alpha.to_bits(),
        stops_addr: std::sync::Arc::as_ptr(stops) as usize,
        stops_len: stops.len() as u32,
    };
    lookup_or_build_gradient(key, stops, || {
        let (colors, positions) = build_colors_positions(stops, global_alpha)?;
        gradient_shader::linear(
            (Point::new(x0, y0), Point::new(x1, y1)),
            gradient_shader::GradientShaderColors::Colors(&colors),
            Some(&positions[..]),
            TileMode::Clamp,
            None,
            None,
        )
    })
}

/// Fetch or build a radial gradient (two-point conical under the hood,
/// matching Canvas 2D's `createRadialGradient`).
pub fn get_or_build_radial_gradient(
    x0: f32,
    y0: f32,
    r0: f32,
    x1: f32,
    y1: f32,
    r1: f32,
    stops: &std::sync::Arc<Vec<GradientStop>>,
    global_alpha: f32,
) -> Option<Shader> {
    let key = GradientKey {
        kind: 1,
        geom_bits: [
            x0.to_bits(),
            y0.to_bits(),
            r0.to_bits(),
            x1.to_bits(),
            y1.to_bits(),
            r1.to_bits(),
            0,
        ],
        alpha_bits: global_alpha.to_bits(),
        stops_addr: std::sync::Arc::as_ptr(stops) as usize,
        stops_len: stops.len() as u32,
    };
    lookup_or_build_gradient(key, stops, || {
        let (colors, positions) = build_colors_positions(stops, global_alpha)?;
        gradient_shader::two_point_conical(
            Point::new(x0, y0),
            r0,
            Point::new(x1, y1),
            r1,
            gradient_shader::GradientShaderColors::Colors(&colors),
            Some(&positions[..]),
            TileMode::Clamp,
            None,
            None,
        )
    })
}

/// Fetch or build a conic (sweep) gradient.
pub fn get_or_build_conic_gradient(
    cx: f32,
    cy: f32,
    start_angle_rad: f32,
    stops: &std::sync::Arc<Vec<GradientStop>>,
    global_alpha: f32,
) -> Option<Shader> {
    let key = GradientKey {
        kind: 2,
        geom_bits: [
            cx.to_bits(),
            cy.to_bits(),
            start_angle_rad.to_bits(),
            0, 0, 0, 0,
        ],
        alpha_bits: global_alpha.to_bits(),
        stops_addr: std::sync::Arc::as_ptr(stops) as usize,
        stops_len: stops.len() as u32,
    };
    lookup_or_build_gradient(key, stops, || {
        let (colors, positions) = build_colors_positions(stops, global_alpha)?;
        let start_deg = start_angle_rad.to_degrees();
        let end_deg = start_deg + 360.0;
        gradient_shader::sweep(
            Point::new(cx, cy),
            gradient_shader::GradientShaderColors::Colors(&colors),
            Some(&positions[..]),
            TileMode::Clamp,
            Some((start_deg, end_deg)),
            None,
            None,
        )
    })
}

#[inline]
fn lookup_or_build_gradient<F: FnOnce() -> Option<Shader>>(
    key: GradientKey,
    stops: &std::sync::Arc<Vec<GradientStop>>,
    build: F,
) -> Option<Shader> {
    GRADIENT_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if let Some(entry) = cache.get(&key) {
            // Verify the snapshot against the current stops to guard
            // against Arc address reuse.  Typical stop counts are
            // 2-5, so slice compare is cheap.
            if entry.stops_snapshot.as_slice() == stops.as_slice() {
                crate::render_diagnostics::hit_gradient_cache();
                return Some(entry.shader.clone());
            }
            // Fallthrough: collision.  We'll build + overwrite.
        }
        crate::render_diagnostics::miss_gradient_cache();
        let shader = build()?;
        cache.put(
            key,
            GradientEntry {
                stops_snapshot: stops.as_slice().to_vec(),
                shader: shader.clone(),
            },
        );
        Some(shader)
    })
}

#[inline]
fn build_colors_positions(
    stops: &[GradientStop],
    global_alpha: f32,
) -> Option<(Vec<skia_safe::Color>, Vec<f32>)> {
    if stops.len() < 2 {
        return None;
    }
    // Same modulation the old `stops_to_colors_positions` did: bake
    // globalAlpha into each stop's colour so Skia's gradient
    // samples pick it up automatically.
    let colors = stops
        .iter()
        .map(|s| {
            super::color::to_sk_color4f_modulated(s.color, global_alpha).to_color()
        })
        .collect::<Vec<_>>();
    let positions = stops.iter().map(|s| s.offset.clamp(0.0, 1.0)).collect();
    Some((colors, positions))
}

/// Empty the caches.  Never strictly necessary (entries are bounded)
/// but useful from tests that want deterministic refcount assertions
/// or from a future `freeGpuResources` integration.
#[allow(dead_code)]
pub fn clear_all() {
    SHADOW_CACHE.with(|c| c.borrow_mut().clear());
    DASH_CACHE.with(|c| c.borrow_mut().clear());
    GRADIENT_CACHE.with(|c| c.borrow_mut().clear());
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

    /// Alias to the crate-wide diagnostics test guard.  See
    /// [`crate::render_diagnostics::TEST_GUARD`] for the rationale
    /// — any test whose production path touches the diagnostics
    /// SINK must take this mutex, otherwise cross-module parallel
    /// tests stomp on each other's metric assertions.
    fn guard() -> std::sync::MutexGuard<'static, ()> {
        crate::render_diagnostics::test_guard()
    }

    #[test]
    fn shadow_cache_hits_on_identical_params() {
        let _g = guard();
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
        let _g = guard();
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
        let _g = guard();
        clear_all();
        let _ = get_or_build_dash(&[4.0, 2.0], 0.0).unwrap();
        let _ = get_or_build_dash(&[4.0, 2.0], 0.0).unwrap();
        DASH_CACHE.with(|c| {
            assert_eq!(c.borrow().len(), 1);
        });
    }

    #[test]
    fn dash_cache_preserves_interval_order() {
        let _g = guard();
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
        let _g = guard();
        clear_all();
        assert!(get_or_build_dash(&[], 0.0).is_none());
    }

    #[test]
    fn dash_cache_distinguishes_phase() {
        let _g = guard();
        clear_all();
        let _ = get_or_build_dash(&[4.0, 2.0], 0.0).unwrap();
        let _ = get_or_build_dash(&[4.0, 2.0], 1.0).unwrap();
        DASH_CACHE.with(|c| {
            assert_eq!(c.borrow().len(), 2);
        });
    }

    // ---- metrics wiring (P1-3a/b + P1-5) --------------------------

    /// Helper: run `body` with a fresh DebugStats sink installed
    /// and cleaned up afterwards.  Callers must hold the module's
    /// `TEST_GUARD` mutex so concurrent effect_cache tests don't
    /// bump counters on the installed sink.
    fn with_sink<F: FnOnce(&std::sync::Arc<shared::stats::DebugStats>)>(f: F) {
        use crate::render_diagnostics;
        render_diagnostics::uninstall_for_tests();
        let stats = std::sync::Arc::new(shared::stats::DebugStats::default());
        render_diagnostics::install(stats.clone());
        f(&stats);
        render_diagnostics::uninstall_for_tests();
    }

    #[test]
    fn shadow_cache_miss_and_hit_each_bump_respective_metric() {
        let _g = guard();
        with_sink(|stats| {
            use std::sync::atomic::Ordering;
            clear_all();

            // First call: miss.
            let _ = get_or_build_drop_shadow(0xFF00_0000, 4.0, 4.0, 2.0, 2.0).unwrap();
            assert_eq!(stats.shadow_filter_misses.load(Ordering::Relaxed), 1);
            assert_eq!(stats.shadow_filter_hits.load(Ordering::Relaxed), 0);

            // Second call with same params: hit.
            let _ = get_or_build_drop_shadow(0xFF00_0000, 4.0, 4.0, 2.0, 2.0).unwrap();
            assert_eq!(stats.shadow_filter_misses.load(Ordering::Relaxed), 1);
            assert_eq!(stats.shadow_filter_hits.load(Ordering::Relaxed), 1);
        });
    }

    #[test]
    fn dash_cache_miss_and_hit_each_bump_respective_metric() {
        let _g = guard();
        with_sink(|stats| {
            use std::sync::atomic::Ordering;
            clear_all();

            let _ = get_or_build_dash(&[4.0, 2.0], 0.0).unwrap();
            assert_eq!(stats.dash_effect_misses.load(Ordering::Relaxed), 1);
            assert_eq!(stats.dash_effect_hits.load(Ordering::Relaxed), 0);

            let _ = get_or_build_dash(&[4.0, 2.0], 0.0).unwrap();
            assert_eq!(stats.dash_effect_misses.load(Ordering::Relaxed), 1);
            assert_eq!(stats.dash_effect_hits.load(Ordering::Relaxed), 1);
        });
    }

    #[test]
    fn distinct_params_all_register_as_misses() {
        let _g = guard();
        with_sink(|stats| {
            use std::sync::atomic::Ordering;
            clear_all();

            // Three distinct shadow tuples ⇒ 3 misses, 0 hits.
            let _ = get_or_build_drop_shadow(0xFF00_0000, 4.0, 4.0, 2.0, 2.0).unwrap();
            let _ = get_or_build_drop_shadow(0xFFFF_0000, 4.0, 4.0, 2.0, 2.0).unwrap();
            let _ = get_or_build_drop_shadow(0xFF00_0000, 8.0, 8.0, 2.0, 2.0).unwrap();
            assert_eq!(stats.shadow_filter_misses.load(Ordering::Relaxed), 3);
            assert_eq!(stats.shadow_filter_hits.load(Ordering::Relaxed), 0);
        });
    }

    // ---- Gradient cache -------------------------------------------

    fn simple_stops() -> std::sync::Arc<Vec<GradientStop>> {
        use shared::protocol::color::Color as ProtocolColor;
        std::sync::Arc::new(vec![
            GradientStop {
                offset: 0.0,
                color: ProtocolColor::rgb(255, 0, 0),
            },
            GradientStop {
                offset: 1.0,
                color: ProtocolColor::rgb(0, 0, 255),
            },
        ])
    }

    #[test]
    fn linear_gradient_hits_on_identical_params_and_same_arc() {
        let _g = guard();
        clear_all();
        let stops = simple_stops();
        let _ =
            get_or_build_linear_gradient(0.0, 0.0, 100.0, 0.0, &stops, 1.0).expect("build");
        // Same Arc + same geometry must hit.
        let _ =
            get_or_build_linear_gradient(0.0, 0.0, 100.0, 0.0, &stops, 1.0).expect("build");
        GRADIENT_CACHE.with(|c| {
            assert_eq!(c.borrow().len(), 1, "identical request must not grow cache");
        });
    }

    #[test]
    fn linear_gradient_distinguishes_geometry() {
        let _g = guard();
        clear_all();
        let stops = simple_stops();
        let _ = get_or_build_linear_gradient(0.0, 0.0, 100.0, 0.0, &stops, 1.0).unwrap();
        let _ = get_or_build_linear_gradient(0.0, 0.0, 100.0, 50.0, &stops, 1.0).unwrap();
        let _ = get_or_build_linear_gradient(10.0, 0.0, 100.0, 0.0, &stops, 1.0).unwrap();
        GRADIENT_CACHE.with(|c| {
            assert_eq!(c.borrow().len(), 3);
        });
    }

    #[test]
    fn gradient_distinguishes_global_alpha() {
        let _g = guard();
        clear_all();
        let stops = simple_stops();
        let _ = get_or_build_linear_gradient(0.0, 0.0, 100.0, 0.0, &stops, 1.0).unwrap();
        let _ = get_or_build_linear_gradient(0.0, 0.0, 100.0, 0.0, &stops, 0.5).unwrap();
        GRADIENT_CACHE.with(|c| {
            assert_eq!(c.borrow().len(), 2);
        });
    }

    #[test]
    fn radial_and_conic_gradient_build() {
        let _g = guard();
        clear_all();
        let stops = simple_stops();
        assert!(
            get_or_build_radial_gradient(0.0, 0.0, 10.0, 50.0, 50.0, 100.0, &stops, 1.0)
                .is_some()
        );
        assert!(get_or_build_conic_gradient(50.0, 50.0, 0.0, &stops, 1.0).is_some());
    }

    #[test]
    fn gradient_cache_miss_and_hit_each_bump_metric() {
        let _g = guard();
        with_sink(|stats| {
            use std::sync::atomic::Ordering;
            clear_all();
            let stops = simple_stops();

            // First call: miss.
            let _ = get_or_build_linear_gradient(0.0, 0.0, 100.0, 0.0, &stops, 1.0)
                .expect("build");
            assert_eq!(stats.gradient_misses.load(Ordering::Relaxed), 1);
            assert_eq!(stats.gradient_hits.load(Ordering::Relaxed), 0);

            // Second call with same Arc + same geometry: hit.
            let _ = get_or_build_linear_gradient(0.0, 0.0, 100.0, 0.0, &stops, 1.0)
                .expect("build");
            assert_eq!(stats.gradient_misses.load(Ordering::Relaxed), 1);
            assert_eq!(stats.gradient_hits.load(Ordering::Relaxed), 1);
        });
    }
}
