//! Process-global image id allocator.
//!
//! Image ids are integers handed out to identify GPU texture slots
//! owned by `graphics::canvas::backend::gl::ImageStore`.  The store
//! lives on the render thread; the JS thread allocates an id on
//! every `new Image()`.
//!
//! Historically this allocation went through `op_create_image`, a
//! sync render-thread RTT — which only existed because the counter
//! lived inside the per-host `ImageStore`.  That RTT became a major
//! head-of-line stall: a busy render thread (e.g. processing a
//! cocos shop scene's first FramePacket, ~700ms) blocked every
//! `new Image()` for the same duration, even though the operation
//! is a pure counter bump.
//!
//! Promoting the counter to a process-global atomic lets both the JS
//! thread and the render thread allocate locally with no
//! synchronisation, and guarantees uniqueness across all hosts in
//! the process — ids never collide between sessions, which is a
//! nice diagnostic property even though no code path relies on it.

use std::sync::atomic::{AtomicU32, Ordering};

/// Monotonic image id counter.  Starts at 1 so `0` remains the
/// "no image" sentinel any future caller can rely on.  `u32` gives
/// 4 billion ids over a process lifetime, more than any realistic
/// session can exhaust.
static NEXT_IMAGE_ID: AtomicU32 = AtomicU32::new(1);

/// Reserve a fresh image id.  Always `> 0`.  Safe to call from any
/// thread; uses `Relaxed` because ids carry no ordering
/// relationship with other memory state.
#[inline]
pub fn next_image_id() -> u32 {
    NEXT_IMAGE_ID.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_monotonic_and_nonzero() {
        let a = next_image_id();
        let b = next_image_id();
        assert!(a > 0);
        assert!(b > a);
    }
}
