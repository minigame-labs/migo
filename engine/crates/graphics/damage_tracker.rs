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

pub struct DamageTracker {
    surface_size: (i32, i32),
    dirty: Option<(i32, i32, i32, i32)>,
    force_full: bool,
}

impl DamageTracker {
    pub fn new(surface_size: (i32, i32)) -> Self {
        Self {
            surface_size,
            dirty: None,
            force_full: false,
        }
    }

    pub fn mark_rect(&mut self, rect: (i32, i32, i32, i32)) {
        let (x, y, width, height) = rect;
        if width <= 0 || height <= 0 {
            return;
        }

        self.dirty = Some(match self.dirty {
            Some((cur_x, cur_y, cur_width, cur_height)) => {
                let left = cur_x.min(x);
                let top = cur_y.min(y);
                let right = (cur_x + cur_width).max(x + width);
                let bottom = (cur_y + cur_height).max(y + height);
                (left, top, right - left, bottom - top)
            }
            None => rect,
        });
    }

    pub fn mark_requires_full_redraw(&mut self) {
        self.force_full = true;
    }

    pub fn resolve(&self) -> ResolvedDamage {
        if self.force_full {
            return ResolvedDamage::FullSurface;
        }

        let (surface_width, surface_height) = self.surface_size;
        if surface_width <= 0 || surface_height <= 0 {
            return ResolvedDamage::FullSurface;
        }

        let Some((x, y, width, height)) = self.dirty else {
            return ResolvedDamage::FullSurface;
        };

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

    // ── DamageHistory tests ──

    #[test]
    fn age_1_returns_current_frame_damage() {
        let history = DamageHistory::new();
        let current = ResolvedDamage::Partial { x: 10, y: 20, width: 100, height: 50 };
        assert_eq!(history.resolve_with_age(current, 1), current);
    }

    #[test]
    fn age_0_returns_full_surface() {
        let history = DamageHistory::new();
        let current = ResolvedDamage::Partial { x: 10, y: 20, width: 100, height: 50 };
        assert_eq!(history.resolve_with_age(current, 0), ResolvedDamage::FullSurface);
    }

    #[test]
    fn age_exceeds_history_returns_full_surface() {
        let history = DamageHistory::new(); // empty
        let current = ResolvedDamage::Partial { x: 0, y: 0, width: 50, height: 50 };
        assert_eq!(history.resolve_with_age(current, 2), ResolvedDamage::FullSurface);
    }

    #[test]
    fn age_2_unions_current_with_previous_frame() {
        let mut history = DamageHistory::new();
        history.push(ResolvedDamage::Partial { x: 200, y: 300, width: 100, height: 100 });

        let current = ResolvedDamage::Partial { x: 10, y: 20, width: 50, height: 50 };
        let result = history.resolve_with_age(current, 2);
        // Union of (10,20,50,50) and (200,300,100,100) = (10,20,290,380)
        assert_eq!(result, ResolvedDamage::Partial { x: 10, y: 20, width: 290, height: 380 });
    }

    #[test]
    fn age_3_unions_current_with_two_previous_frames() {
        let mut history = DamageHistory::new();
        history.push(ResolvedDamage::Partial { x: 0, y: 0, width: 10, height: 10 });
        history.push(ResolvedDamage::Partial { x: 500, y: 500, width: 100, height: 100 });

        let current = ResolvedDamage::Partial { x: 250, y: 250, width: 50, height: 50 };
        let result = history.resolve_with_age(current, 3);
        // Union of (0,0,10,10), (500,500,100,100), (250,250,50,50)
        assert_eq!(result, ResolvedDamage::Partial { x: 0, y: 0, width: 600, height: 600 });
    }

    #[test]
    fn historical_full_surface_poisons_result() {
        let mut history = DamageHistory::new();
        history.push(ResolvedDamage::FullSurface);

        let current = ResolvedDamage::Partial { x: 10, y: 10, width: 50, height: 50 };
        assert_eq!(history.resolve_with_age(current, 2), ResolvedDamage::FullSurface);
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
        history.push(ResolvedDamage::Partial { x: 0, y: 0, width: 10, height: 10 });
        history.clear();
        assert_eq!(history.ring.len(), 0);
    }
}
