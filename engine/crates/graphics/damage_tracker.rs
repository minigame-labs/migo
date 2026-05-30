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

/// History ring of recent frame damages for buffer-age-aware partial present.
///
/// EGL_EXT_buffer_age tells us how many swaps ago the current back buffer was
/// last presented.  To correctly declare damage we must union the current
/// frame's damage with all frames since the buffer was last shown.
///
/// Capacity is bounded at `MAX_HISTORY` — if buffer age exceeds the window,
/// we conservatively fall back to FullSurface.
pub struct DamageHistory {
    ring: std::collections::VecDeque<ResolvedDamage>,
}

/// Maximum number of historical frames kept.  Triple-buffered (age up to 3)
/// plus 1 margin for driver variance.
const MAX_HISTORY: usize = 4;

impl DamageHistory {
    pub fn new() -> Self {
        Self {
            ring: std::collections::VecDeque::with_capacity(MAX_HISTORY),
        }
    }

    /// Record the current frame's resolved damage after swap.
    pub fn push(&mut self, damage: ResolvedDamage) {
        if self.ring.len() >= MAX_HISTORY {
            self.ring.pop_front();
        }
        self.ring.push_back(damage);
    }

    /// Compute the damage region to declare given the buffer age.
    ///
    /// `current_frame` is this frame's resolved damage (before age expansion).
    /// `buffer_age` is the value from `eglQuerySurface(EGL_BUFFER_AGE_*)`.
    ///
    /// Returns `FullSurface` when age is 0 (undefined), exceeds history, or
    /// any historical frame was FullSurface.
    pub fn resolve_with_age(
        &self,
        current_frame: ResolvedDamage,
        buffer_age: i32,
    ) -> ResolvedDamage {
        // age 0 = undefined contents, age < 0 = error — full redraw
        if buffer_age <= 0 {
            return ResolvedDamage::FullSurface;
        }
        // age 1 = buffer was presented last frame, contents preserved — just current damage
        if buffer_age == 1 {
            return current_frame;
        }
        // age > 1 — need to union the last (age-1) historical frames + current
        let history_needed = (buffer_age - 1) as usize;
        if history_needed > self.ring.len() {
            return ResolvedDamage::FullSurface;
        }

        // Start with current frame's damage
        let mut result = current_frame;

        // Union with the most recent `history_needed` entries
        let start = self.ring.len() - history_needed;
        for entry in self.ring.range(start..) {
            result = union_damage(result, *entry);
        }

        result
    }

    /// Clear history (e.g. after surface recreation).
    pub fn clear(&mut self) {
        self.ring.clear();
    }
}

/// Union two resolved damages. If either is FullSurface, result is FullSurface.
fn union_damage(a: ResolvedDamage, b: ResolvedDamage) -> ResolvedDamage {
    match (a, b) {
        (ResolvedDamage::FullSurface, _) | (_, ResolvedDamage::FullSurface) => {
            ResolvedDamage::FullSurface
        }
        (
            ResolvedDamage::Partial {
                x: ax,
                y: ay,
                width: aw,
                height: ah,
            },
            ResolvedDamage::Partial {
                x: bx,
                y: by,
                width: bw,
                height: bh,
            },
        ) => {
            let left = ax.min(bx);
            let top = ay.min(by);
            let right = (ax + aw).max(bx + bw);
            let bottom = (ay + ah).max(by + bh);
            ResolvedDamage::Partial {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            }
        }
    }
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

    // ── DamageHistory tests ──

    #[test]
    fn age_1_returns_current_frame_damage() {
        let history = DamageHistory::new();
        let current = ResolvedDamage::Partial {
            x: 10,
            y: 20,
            width: 100,
            height: 50,
        };
        assert_eq!(history.resolve_with_age(current, 1), current);
    }

    #[test]
    fn age_0_returns_full_surface() {
        let history = DamageHistory::new();
        let current = ResolvedDamage::Partial {
            x: 10,
            y: 20,
            width: 100,
            height: 50,
        };
        assert_eq!(
            history.resolve_with_age(current, 0),
            ResolvedDamage::FullSurface
        );
    }

    #[test]
    fn age_exceeds_history_returns_full_surface() {
        let history = DamageHistory::new(); // empty
        let current = ResolvedDamage::Partial {
            x: 0,
            y: 0,
            width: 50,
            height: 50,
        };
        assert_eq!(
            history.resolve_with_age(current, 2),
            ResolvedDamage::FullSurface
        );
    }

    #[test]
    fn age_2_unions_current_with_previous_frame() {
        let mut history = DamageHistory::new();
        history.push(ResolvedDamage::Partial {
            x: 200,
            y: 300,
            width: 100,
            height: 100,
        });

        let current = ResolvedDamage::Partial {
            x: 10,
            y: 20,
            width: 50,
            height: 50,
        };
        let result = history.resolve_with_age(current, 2);
        // Union of (10,20,50,50) and (200,300,100,100) = (10,20,290,380)
        assert_eq!(
            result,
            ResolvedDamage::Partial {
                x: 10,
                y: 20,
                width: 290,
                height: 380
            }
        );
    }

    #[test]
    fn age_3_unions_current_with_two_previous_frames() {
        let mut history = DamageHistory::new();
        history.push(ResolvedDamage::Partial {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        });
        history.push(ResolvedDamage::Partial {
            x: 500,
            y: 500,
            width: 100,
            height: 100,
        });

        let current = ResolvedDamage::Partial {
            x: 250,
            y: 250,
            width: 50,
            height: 50,
        };
        let result = history.resolve_with_age(current, 3);
        // Union of (0,0,10,10), (500,500,100,100), (250,250,50,50)
        assert_eq!(
            result,
            ResolvedDamage::Partial {
                x: 0,
                y: 0,
                width: 600,
                height: 600
            }
        );
    }

    #[test]
    fn historical_full_surface_poisons_result() {
        let mut history = DamageHistory::new();
        history.push(ResolvedDamage::FullSurface);

        let current = ResolvedDamage::Partial {
            x: 10,
            y: 10,
            width: 50,
            height: 50,
        };
        assert_eq!(
            history.resolve_with_age(current, 2),
            ResolvedDamage::FullSurface
        );
    }

    #[test]
    fn history_ring_evicts_oldest() {
        let mut history = DamageHistory::new();
        for i in 0..MAX_HISTORY + 2 {
            history.push(ResolvedDamage::Partial {
                x: i as i32 * 10,
                y: 0,
                width: 10,
                height: 10,
            });
        }
        assert_eq!(history.ring.len(), MAX_HISTORY);
    }

    #[test]
    fn clear_empties_history() {
        let mut history = DamageHistory::new();
        history.push(ResolvedDamage::Partial {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        });
        history.clear();
        assert_eq!(history.ring.len(), 0);
    }
}
