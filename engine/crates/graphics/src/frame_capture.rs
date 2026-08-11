//! One-shot framebuffer capture for the headless dev player (offscreen PNG).
//!
//! Correctness first: the render thread reads the default framebuffer (FBO 0)
//! **after** the DrawingBuffer→surface blit and **before** `eglSwapBuffers`,
//! while the onscreen GL context is current and the back buffer is still valid
//! — so the captured pixels are exactly what is about to be presented,
//! independent of the game's internals.
//!
//! Performance: the render thread pays only a single atomic load per presented
//! frame (`requested_seq`); the `glReadPixels` cost is incurred once, when a
//! capture has actually been requested.
//!
//! This is a diagnostic/dev-tool hook (the player). It never runs unless
//! `request()` is called, so it has no effect on shipping render behaviour.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use glow::HasContext;

/// Which request the render thread is capturing for; 0 means none.
///
/// A counter rather than a flag, because "capture the next presented frame" is a
/// claim no earlier frame may satisfy. A resize acceptance run requests twice --
/// once at start-up and once after the transition -- and under a flag the second
/// request was already satisfied by the frame the first left in the slot, so a
/// resize that never presented wrote the pre-resize picture and exited
/// successfully. Stamping each frame with the request it answers makes that
/// unrepresentable; clearing the slot inside `request` would only narrow it,
/// since a capture already between its `glReadPixels` and its store still lands
/// afterwards.
static REQUEST_SEQ: AtomicU64 = AtomicU64::new(0);
static RESULT: Mutex<Option<(u64, CapturedFrame)>> = Mutex::new(None);

/// A captured frame. `rgba_bottom_up` is tightly packed RGBA8 in GL row order
/// (bottom-up); consumers flip to top-down for PNG.
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba_bottom_up: Vec<u8>,
}

/// Request a one-shot capture of the next presented frame.
///
/// Called again before [`take`] it supersedes the earlier request: from then on
/// only a newly presented frame can satisfy it.
pub fn request() {
    REQUEST_SEQ.fetch_add(1, Ordering::AcqRel);
}

/// Stop capturing and take the most recent frame presented since the latest
/// [`request`]. A frame that answered an earlier request is discarded rather
/// than returned, so a run that presented nothing after its last request gets
/// `None` instead of a stale picture.
pub fn take() -> Option<CapturedFrame> {
    let wanted = REQUEST_SEQ.swap(0, Ordering::AcqRel);
    let captured = RESULT.lock().expect("frame_capture result mutex").take();
    captured
        .filter(|(seq, _)| *seq == wanted)
        .map(|(_, frame)| frame)
}

#[inline]
fn requested_seq() -> u64 {
    REQUEST_SEQ.load(Ordering::Acquire)
}

/// Read FBO 0 into the result slot while a capture is pending, overwriting any
/// previous frame so the slot always holds the latest presented frame. MUST be
/// called on the render thread with the onscreen context current, after the
/// final blit and before `eglSwapBuffers`. On the common path (no capture
/// requested — e.g. every shipping Android frame) it costs only one atomic load.
pub(crate) fn capture_default_fbo(gl: &glow::Context, width: u32, height: u32) {
    let seq = requested_seq();
    if seq == 0 || width == 0 || height == 0 {
        return;
    }
    let mut buf = vec![0u8; width as usize * height as usize * 4];
    unsafe {
        // Read from the presented surface (default framebuffer), not the
        // DrawingBuffer that the blit left bound as the read target.
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
        gl.read_pixels(
            0,
            0,
            width as i32,
            height as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut buf)),
        );
    }
    let mut slot = RESULT.lock().expect("frame_capture result mutex");
    // A request that arrived while this frame was being read owns the slot now:
    // this frame predates it, and is exactly the stale capture `take` refuses.
    if slot.as_ref().is_some_and(|(stored, _)| *stored > seq) {
        return;
    }
    *slot = Some((
        seq,
        CapturedFrame {
            width,
            height,
            rgba_bottom_up: buf,
        },
    ));
    // Intentionally do NOT clear the request here: keep the latest frame until
    // the consumer calls take(), so blank warmup frames are replaced by content.
}
