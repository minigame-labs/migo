use smallvec::SmallVec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedDamage {
    FullSurface,
    Partial {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
}

/// Maximum number of discrete dirty rectangles kept before the
/// tracker collapses them to a single AABB.  Set to 4 because
/// `eglSetDamageRegionKHR` accepts an arbitrary-length array but
/// every additional rect adds a tile-rejection check on the GPU
/// side; 4 is the sweet spot for common UI updates (status bar
/// tick, HUD value change, notification slide-in, cursor blink)
/// without spamming the driver.
pub const MAX_DAMAGE_RECTS: usize = 4;

pub struct DamageTracker {
    surface_size: (i32, i32),
    /// Current frame's dirty list.  Populated by `mark_rect`; kept
    /// as discrete rectangles up to `MAX_DAMAGE_RECTS` and then
    /// collapsed to a single AABB on overflow.  `None`-equivalent
    /// is an empty vec.
    dirty: SmallVec<[(i32, i32, i32, i32); MAX_DAMAGE_RECTS]>,
    force_full: bool,
}

impl DamageTracker {
    pub fn new(surface_size: (i32, i32)) -> Self {
        Self {
            surface_size,
            dirty: SmallVec::new(),
            force_full: false,
        }
    }

    /// Record a damaged rect. Rejects geometry the downstream arithmetic cannot
    /// represent, which is the whole of its validation duty.
    ///
    /// **Rejecting an unrepresentable edge here is what keeps `resolve`,
    /// `dirty_aabb` and `collapse_to_aabb` free to add `x + width` in plain
    /// `i32`.** They all do, in six places, and the input is not bounded by
    /// anything else: `canvas2d_dispatcher::classify_draw_damage` derives these
    /// coordinates from a CTM-transformed rect with `.floor() as i32`, and a
    /// float cast in Rust *saturates*, so a pathological transform yields
    /// `i32::MAX` rather than a wrap or a NaN.
    ///
    /// Left alone, that overflows. In release the wrap happened to be harmless —
    /// `right` came out hugely negative, the `left >= right` emptiness check
    /// fired, and `resolve` returned `FullSurface`, which is conservative. In
    /// debug it is an arithmetic-overflow panic on the render thread, driven by
    /// content-controlled input, and the engine does get built in debug (the
    /// `verify-canvas-follows-surface` gate runs a dev-profile player).
    ///
    /// Same strategy as `present_damage::DamageRect::new`, which checks its
    /// edges in `i64` for the same reason: put the invariant at the one entry
    /// point so every consumer downstream inherits it.
    pub fn mark_rect(&mut self, rect: (i32, i32, i32, i32)) {
        let (x, y, width, height) = rect;
        if width <= 0 || height <= 0 {
            return;
        }
        // `i64` so the check itself cannot overflow.
        let right = i64::from(x) + i64::from(width);
        let bottom = i64::from(y) + i64::from(height);
        if right > i64::from(i32::MAX) || bottom > i64::from(i32::MAX) {
            // Unrepresentable extent. Escalating to a full redraw is the honest
            // answer — a rect this large covers the surface anyway, and it keeps
            // every `x + width` below in range.
            self.force_full = true;
            return;
        }
        if self.dirty.len() < MAX_DAMAGE_RECTS {
            self.dirty.push(rect);
            return;
        }
        // Past the cap: collapse everything (including the new
        // rect) into a single AABB so the per-frame memory and
        // driver cost stays bounded.  Subsequent `mark_rect` calls
        // will hit this path and keep growing the AABB in place
        // because after the collapse `self.dirty.len() == 1` is
        // below the cap - but the first rect is now the AABB, so
        // further appends would reintroduce fragmentation.  Fix
        // that by treating overflow as sticky: once we've
        // collapsed, future calls keep unioning into the single
        // AABB slot rather than growing the list again.
        if self.dirty.len() >= MAX_DAMAGE_RECTS {
            let collapsed = collapse_to_aabb(&self.dirty, rect);
            self.dirty.clear();
            self.dirty.push(collapsed);
        }
    }

    pub fn mark_requires_full_redraw(&mut self) {
        self.force_full = true;
    }

    /// Resolve the accumulated damage to a single AABB
    /// [`ResolvedDamage`].  Retained for consumers (swap path,
    /// stats) that only need the bounding box; callers that want
    /// the multi-rect form should use [`Self::resolve_rects`].
    pub fn resolve(&self) -> ResolvedDamage {
        if self.force_full {
            return ResolvedDamage::FullSurface;
        }

        let (surface_width, surface_height) = self.surface_size;
        if surface_width <= 0 || surface_height <= 0 {
            return ResolvedDamage::FullSurface;
        }

        let Some(aabb) = dirty_aabb(&self.dirty) else {
            return ResolvedDamage::FullSurface;
        };
        let (x, y, width, height) = aabb;

        let left = x.max(0);
        let top = y.max(0);
        let right = (x + width).min(surface_width);
        let bottom = (y + height).min(surface_height);

        if left >= right || top >= bottom {
            return ResolvedDamage::FullSurface;
        }

        let width = right - left;
        let height = bottom - top;
        if width >= surface_width && height >= surface_height {
            return ResolvedDamage::FullSurface;
        }

        ResolvedDamage::Partial {
            x: left,
            y: top,
            width,
            height,
        }
    }

    /// Return the accumulated dirty rectangles (up to
    /// [`MAX_DAMAGE_RECTS`]) clipped to the surface bounds, or
    /// `None` when the frame requires a full redraw.
    ///
    /// Unlike [`Self::resolve`], this preserves the per-rect
    /// granularity so partial-update consumers (e.g.
    /// `eglSetDamageRegionKHR` with `n_rects > 1`) can let the
    /// driver reject tiles that no rect covers.
    pub fn resolve_rects(&self) -> Option<SmallVec<[(i32, i32, i32, i32); MAX_DAMAGE_RECTS]>> {
        if self.force_full {
            return None;
        }
        let (surface_width, surface_height) = self.surface_size;
        if surface_width <= 0 || surface_height <= 0 || self.dirty.is_empty() {
            return None;
        }
        let mut out: SmallVec<[(i32, i32, i32, i32); MAX_DAMAGE_RECTS]> = SmallVec::new();
        for &(x, y, w, h) in &self.dirty {
            let left = x.max(0);
            let top = y.max(0);
            let right = (x + w).min(surface_width);
            let bottom = (y + h).min(surface_height);
            if left >= right || top >= bottom {
                continue;
            }
            out.push((left, top, right - left, bottom - top));
        }
        if out.is_empty() {
            return None;
        }
        // If any single rect already covers the whole surface the
        // caller saves nothing by issuing per-rect damage: let them
        // fall back to FullSurface by returning `None`.  Using a
        // strict `>=` match lets us honour the same semantics as
        // `resolve()`.
        if out
            .iter()
            .any(|&(_, _, w, h)| w >= surface_width && h >= surface_height)
        {
            return None;
        }
        Some(out)
    }
}

/// Compute the bounding AABB of a list of rects.  `None` when the
/// input is empty.
fn dirty_aabb(rects: &[(i32, i32, i32, i32)]) -> Option<(i32, i32, i32, i32)> {
    let mut it = rects.iter().copied();
    let (mut left, mut top, mut right, mut bottom) = {
        let (x, y, w, h) = it.next()?;
        (x, y, x + w, y + h)
    };
    for (x, y, w, h) in it {
        left = left.min(x);
        top = top.min(y);
        right = right.max(x + w);
        bottom = bottom.max(y + h);
    }
    Some((left, top, right - left, bottom - top))
}

/// Collapse a full dirty list plus one extra rect into a single
/// AABB.  Used by the overflow path in [`DamageTracker::mark_rect`].
fn collapse_to_aabb(
    existing: &[(i32, i32, i32, i32)],
    extra: (i32, i32, i32, i32),
) -> (i32, i32, i32, i32) {
    let (mut left, mut top, mut right, mut bottom) = {
        let (x, y, w, h) = extra;
        (x, y, x + w, y + h)
    };
    for &(x, y, w, h) in existing {
        left = left.min(x);
        top = top.min(y);
        right = right.max(x + w);
        bottom = bottom.max(y + h);
    }
    (left, top, right - left, bottom - top)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A rect whose right or bottom edge overflows `i32` must not reach the
    /// arithmetic**, which adds `x + width` in plain `i32` in six places across
    /// `resolve`, `dirty_aabb` and `collapse_to_aabb`.
    ///
    /// Reachable input, not a hypothetical: `classify_draw_damage` derives these
    /// coordinates from a CTM-transformed rect via `.floor() as i32`, and a float
    /// cast in Rust saturates, so a pathological transform hands over `i32::MAX`.
    ///
    /// In release the wrap was harmless by luck — `right` came out negative, the
    /// emptiness check fired, `FullSurface`. In debug it is an overflow panic on
    /// the render thread. This test runs in both, and in debug it fails by
    /// panicking inside `resolve` if the guard is removed rather than by an
    /// assertion, which is the point.
    #[test]
    fn an_unrepresentable_edge_escalates_instead_of_overflowing() {
        let mut t = DamageTracker::new((1080, 1920));

        // Right edge past i32::MAX.
        t.mark_rect((i32::MAX - 10, 0, 100, 50));
        assert_eq!(
            t.resolve(),
            ResolvedDamage::FullSurface,
            "an unrepresentable rect must escalate to a full redraw"
        );

        // Bottom edge past i32::MAX, on a fresh tracker.
        let mut t = DamageTracker::new((1080, 1920));
        t.mark_rect((0, i32::MAX - 10, 50, 100));
        assert_eq!(t.resolve(), ResolvedDamage::FullSurface);

        // The guard must bound the *edge*, not reject large extents. The largest
        // representable rect gets through `mark_rect` — which is the property
        // under test — and `resolve` then reports `FullSurface` for its own
        // reason: a rect covering the whole surface has nothing to repair
        // partially. Two different escalations that happen to agree, so the
        // interesting assertion is the one below, on a rect that is large but
        // not total.
        let mut t = DamageTracker::new((1080, 1920));
        t.mark_rect((0, 0, i32::MAX, i32::MAX));
        assert_eq!(t.resolve(), ResolvedDamage::FullSurface);

        // Large, representable, and *not* covering the surface: this must clamp
        // to a partial rect. If the edge guard were written against the extent
        // instead of the edge, this rect would be rejected and the frame would
        // repaint in full for nothing.
        let mut t = DamageTracker::new((1080, 1920));
        t.mark_rect((0, 0, i32::MAX, 100));
        assert_eq!(
            t.resolve(),
            ResolvedDamage::Partial {
                x: 0,
                y: 0,
                width: 1080,
                height: 100
            },
            "a representable rect with a huge extent must clamp, not escalate"
        );

        // And an overflowing rect among ordinary ones still escalates rather
        // than being quietly dropped: it covers everything, so the frame does.
        let mut t = DamageTracker::new((1080, 1920));
        t.mark_rect((10, 10, 100, 100));
        t.mark_rect((i32::MAX - 1, 0, 2, 2));
        assert_eq!(t.resolve(), ResolvedDamage::FullSurface);
    }

    /// Past the rect cap the collapse also adds edges, so an overflowing rect
    /// arriving *after* the cap must be stopped by the same guard — the collapse
    /// runs before any emptiness check could catch it.
    #[test]
    fn an_unrepresentable_edge_is_stopped_before_the_collapse() {
        let mut t = DamageTracker::new((1080, 1920));
        for i in 0..MAX_DAMAGE_RECTS {
            t.mark_rect((i as i32 * 10, 0, 5, 5));
        }
        // The next rect takes the collapse path, where `collapse_to_aabb` adds
        // `x + w` for every held rect and the incoming one.
        t.mark_rect((i32::MAX - 1, 0, 2, 2));
        assert_eq!(t.resolve(), ResolvedDamage::FullSurface);
    }

    #[test]
    fn falls_back_to_full_redraw_when_frame_contains_untracked_readback() {
        let mut tracker = DamageTracker::new((1080, 1920));
        tracker.mark_rect((0, 0, 200, 200));
        tracker.mark_requires_full_redraw();

        assert_eq!(tracker.resolve(), ResolvedDamage::FullSurface);
    }

    #[test]
    fn resolve_rects_returns_discrete_rects_up_to_cap() {
        let mut tracker = DamageTracker::new((1080, 1920));
        tracker.mark_rect((10, 20, 30, 40));
        tracker.mark_rect((500, 600, 50, 60));
        tracker.mark_rect((700, 800, 70, 80));
        tracker.mark_rect((900, 1000, 90, 100));

        let rects = tracker.resolve_rects().expect("should have rects");
        assert_eq!(rects.len(), 4);
        assert_eq!(rects[0], (10, 20, 30, 40));
        assert_eq!(rects[3], (900, 1000, 90, 100));
    }

    #[test]
    fn resolve_rects_collapses_past_cap() {
        let mut tracker = DamageTracker::new((1080, 1920));
        for i in 0..MAX_DAMAGE_RECTS {
            tracker.mark_rect((i as i32 * 10, 0, 10, 10));
        }
        // Cap-th entry + the 5th mark should collapse to a single
        // AABB spanning from (0,0) to (4*10 + 10, 10) = (0,0,50,10).
        tracker.mark_rect((40, 0, 10, 10));
        let rects = tracker.resolve_rects().expect("should have rects");
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0], (0, 0, 50, 10));
    }

    #[test]
    fn resolve_rects_returns_none_when_force_full() {
        let mut tracker = DamageTracker::new((1080, 1920));
        tracker.mark_rect((0, 0, 100, 100));
        tracker.mark_requires_full_redraw();
        assert!(tracker.resolve_rects().is_none());
    }

    #[test]
    fn resolve_rects_clips_to_surface_bounds() {
        let mut tracker = DamageTracker::new((100, 100));
        tracker.mark_rect((-10, -10, 50, 50));
        tracker.mark_rect((80, 80, 50, 50));
        let rects = tracker.resolve_rects().expect("should have rects");
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0], (0, 0, 40, 40));
        assert_eq!(rects[1], (80, 80, 20, 20));
    }

    #[test]
    fn resolve_single_aabb_matches_unified_bbox() {
        // When 4 rects are present, `resolve()` still returns one
        // AABB covering all of them - important so existing
        // non-multi-rect callers keep working unchanged.
        let mut tracker = DamageTracker::new((1000, 1000));
        tracker.mark_rect((10, 10, 20, 20));
        tracker.mark_rect((100, 100, 30, 30));
        tracker.mark_rect((500, 500, 50, 50));
        tracker.mark_rect((800, 800, 40, 40));

        match tracker.resolve() {
            ResolvedDamage::Partial {
                x,
                y,
                width,
                height,
            } => {
                assert_eq!(x, 10);
                assert_eq!(y, 10);
                assert_eq!(width, 830); // 800 + 40 - 10
                assert_eq!(height, 830);
            }
            other => panic!("expected Partial, got {:?}", other),
        }
    }
}
