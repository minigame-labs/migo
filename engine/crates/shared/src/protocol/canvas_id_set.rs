//! A small set of canvas ids, gathered without touching the heap.

use smallvec::SmallVec;

use crate::protocol::render_cmd::CanvasId;

/// How many distinct canvases fit before the set spills.
///
/// Sized for the heaviest scene profiled — the shop-open frame that motivated
/// the render thread's phase reorder, at about thirty offscreen Canvas2D
/// labels. Every other caller is far below it: a WebGL batch is issued against
/// one canvas, so its touched set is normally a single entry.
pub const CANVAS_ID_SET_INLINE_CAPACITY: usize = 32;

/// The three per-frame canvas-id sets in the render path were three
/// independently written `HashSet`s, each built and thrown away every frame or
/// every batch, to answer a question about a handful of small integers:
/// which canvases a packet's Canvas2D half addresses, which ones a WebGL batch
/// touched, and which ones still hold unmaterialised 2D work.
///
/// At these sizes a linear scan over an inline array beats hashing on every
/// count that matters — no allocation, no hashing, and the whole set inside a
/// couple of cache lines. One type rather than three call sites' worth of
/// `SmallVec` handling, so the deduplication a set implies is written once and
/// gated once; an inline `SmallVec` that a caller forgets to check before
/// pushing is a set only by intention.
///
/// Past the inline capacity it spills to the heap and stays correct, which is
/// the right failure mode for a fast path: a scene nobody has produced yet
/// costs an allocation rather than a wrong answer.
#[derive(Debug, Default, Clone)]
pub struct CanvasIdSet {
    ids: SmallVec<[CanvasId; CANVAS_ID_SET_INLINE_CAPACITY]>,
}

impl CanvasIdSet {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `id`, reporting whether it was not already present.
    #[inline]
    pub fn insert(&mut self, id: CanvasId) -> bool {
        if self.ids.contains(&id) {
            return false;
        }
        self.ids.push(id);
        true
    }

    #[inline]
    pub fn contains(&self, id: CanvasId) -> bool {
        self.ids.contains(&id)
    }

    /// Empties the set while keeping whatever capacity it reached, so a set
    /// reused across frames stops allocating after the first spill.
    #[inline]
    pub fn clear(&mut self) {
        self.ids.clear();
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Iterates in insertion order.
    ///
    /// Deliberately not a `HashSet`'s arbitrary order: the callers feed these
    /// ids straight into emitted render ops, and an order that varies run to
    /// run would make a packet's contents unreproducible for no benefit.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = CanvasId> + '_ {
        self.ids.iter().copied()
    }
}

impl<'a> IntoIterator for &'a CanvasIdSet {
    type Item = CanvasId;
    type IntoIter = std::iter::Copied<std::slice::Iter<'a, CanvasId>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.ids.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use migo_alloc_probe::{Burst, assert_no_steady_state_allocation};

    const WARMUP: usize = 4;
    const MEASURED: usize = 64;

    #[test]
    fn a_repeated_id_is_held_once_and_reported_as_already_present() {
        let mut set = CanvasIdSet::new();

        assert!(set.insert(7));
        assert!(
            !set.insert(7),
            "a second insert must report the id was present"
        );
        assert_eq!(set.len(), 1);
        assert!(set.contains(7));
    }

    #[test]
    fn iteration_follows_insertion_order() {
        let mut set = CanvasIdSet::new();
        for id in [9, 3, 9, 12, 3] {
            set.insert(id);
        }

        assert_eq!(set.iter().collect::<Vec<_>>(), vec![9, 3, 12]);
    }

    #[test]
    fn clearing_empties_the_set_without_giving_up_its_capacity() {
        let mut set = CanvasIdSet::new();
        for id in 0..(CANVAS_ID_SET_INLINE_CAPACITY as CanvasId + 40) {
            set.insert(id);
        }
        let spilled = set.ids.capacity();
        assert!(
            set.ids.spilled(),
            "fixture must reach the heap to have capacity worth keeping"
        );

        set.clear();

        assert!(set.is_empty());
        assert_eq!(
            set.ids.capacity(),
            spilled,
            "a set reused across frames gave up its allocation and has to grow again"
        );
    }

    /// Past the inline capacity the set must still answer correctly. A spill
    /// that lost entries would tell the render thread a canvas is untouched
    /// when it is, and the Canvas2D context that needed invalidating would keep
    /// drawing against stale Skia state.
    #[test]
    fn a_spilled_set_still_holds_every_id() {
        let count = CANVAS_ID_SET_INLINE_CAPACITY as CanvasId + 40;
        let mut set = CanvasIdSet::new();
        for id in 0..count {
            set.insert(id);
        }

        assert_eq!(set.len(), count as usize);
        for id in 0..count {
            assert!(set.contains(id), "the spill lost canvas {id}");
        }
        assert!(!set.contains(count));
    }

    /// Section 7.3: these sets are built on per-frame and per-batch paths, so
    /// whatever they cost, the engine pays once a frame for as long as it runs.
    /// The three `HashSet`s this type replaced each cost an allocation and a
    /// free there.
    #[test]
    fn gathering_a_frames_canvases_never_reaches_the_heap() {
        // The shape the render path actually produces: a scene's worth of
        // distinct canvases, each addressed more than once.
        assert_no_steady_state_allocation(
            Burst {
                path: "canvas id set: gather one frame's canvases",
                warmup: WARMUP,
                measured: MEASURED,
            },
            |_| {
                let mut set = CanvasIdSet::new();
                for id in 0..24 {
                    set.insert(id);
                    set.insert(id);
                }
                std::hint::black_box(set.contains(23))
            },
        );
    }
}
