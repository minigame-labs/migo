//! One-shot framebuffer capture for the headless dev player (offscreen PNG).
//!
//! Correctness first: the render thread reads the default framebuffer (FBO 0)
//! **after** the DrawingBuffer→surface blit and **before** `eglSwapBuffers`,
//! while the onscreen GL context is current and the back buffer is still valid
//! — so the captured pixels are exactly what is about to be presented,
//! independent of the game's internals.
//!
//! Performance: the render thread pays only a single relaxed atomic load per
//! presented frame (`is_requested`); the `glReadPixels` cost is incurred once,
//! when a capture has actually been requested.
//!
//! This is a diagnostic/dev-tool hook (the player). It never runs unless
//! `request()` is called, so it has no effect on shipping render behaviour.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use glow::HasContext;

static REQUESTED: AtomicBool = AtomicBool::new(false);
static RESULT: Mutex<Option<CapturedFrame>> = Mutex::new(None);

/// A captured frame. `rgba_bottom_up` is tightly packed RGBA8 in GL row order
/// (bottom-up); consumers flip to top-down for PNG.
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba_bottom_up: Vec<u8>,
}

/// Request a one-shot capture of the next presented frame. Idempotent until the
/// render thread fulfils it.
pub fn request() {
    REQUESTED.store(true, Ordering::Release);
}

/// Stop capturing and take the most recently captured frame. Returns the latest
/// present seen since `request()` — so early blank/clear frames are superseded
/// by later frames that contain game content.
pub fn take() -> Option<CapturedFrame> {
    REQUESTED.store(false, Ordering::Release);
    RESULT.lock().expect("frame_capture result mutex").take()
}

#[inline]
fn is_requested() -> bool {
    REQUESTED.load(Ordering::Acquire)
}

/// Read FBO 0 into the result slot while a capture is pending, overwriting any
/// previous frame so the slot always holds the latest presented frame. MUST be
/// called on the render thread with the onscreen context current, after the
/// final blit and before `eglSwapBuffers`. On the common path (no capture
/// requested — e.g. every shipping Android frame) it costs only one atomic load.
pub(crate) fn capture_default_fbo(gl: &glow::Context, width: u32, height: u32) {
    if !is_requested() || width == 0 || height == 0 {
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
    *RESULT.lock().expect("frame_capture result mutex") = Some(CapturedFrame {
        width,
        height,
        rgba_bottom_up: buf,
    });
    // Intentionally do NOT clear REQUESTED here: keep the latest frame until the
    // consumer calls take(), so blank warmup frames are replaced by content.
}
