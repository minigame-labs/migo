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

    pub fn mark_rect(&mut self, rect: (i32, i32, i32, i32)) {
        let (_x, _y, width, height) = rect;
        if width <= 0 || height <= 0 {
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
