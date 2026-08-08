// Two live canvases, drawn to in both orders, on whichever presentation path
// `can_bypass_drawing_buffer` selects.
//
// `bypass-probe` and `blit-probe` differ by one line -- whether a second canvas
// *exists* -- and neither ever draws to it. That is enough to select the path,
// and it is not enough to exercise the reason the selecting condition was
// written for. The recorded reason is that real FBO 0 follows the current EGL
// draw surface, so an onscreen draw issued while an offscreen pbuffer is current
// would land in the pbuffer. A canvas that is never drawn to is never made
// current, so no probe here has ever switched EGL contexts inside a frame.
//
// This one does, twice per frame, and in the order that is hard rather than the
// order that is easy: the offscreen canvas is drawn *last*, so the offscreen
// pbuffer is the current draw surface when the frame ends and presentation has
// to bring the window back by itself.
//
// The offscreen colour is chosen to be a verdict rather than a decoration. It is
// opaque red, which is not the expected colour and not the frozen-first-frame
// colour, so the capture separates three outcomes that a single flat expectation
// would collapse:
//
//   * green  -- the onscreen clear reached the window;
//   * red    -- an offscreen draw reached the window, i.e. "FBO 0" named the
//               pbuffer's twin rather than the window, or the reverse;
//   * blue   -- the engine presented the first frame and then stopped;
//   * empty  -- the onscreen clear went somewhere that is not the window, which
//               is exactly what the dead bypass path used to do.
const canvas = migo.createCanvas();
const gl = canvas.getContext("webgl");

// A second *live* canvas: it has its own WebGL context, and therefore its own
// EGL context and its own pbuffer surface. Drawing to it is what forces
// `make_current_needed` to switch surfaces inside the frame. Held in a used
// binding rather than an unused const because a canvas nothing uses can be
// collected -- and a collected canvas would silently turn this back into
// `bypass-probe`.
const offscreen = migo.createCanvas();
const glOff = offscreen.getContext("webgl");

const R = 0.2;
const G = 0.8;
const B = 0.4;

let frames = 0;

function paint() {
  // Offscreen first, so the onscreen clear that follows has to survive a
  // context switch away and back.
  glOff.clearColor(1.0, 0.0, 0.0, 1.0);
  glOff.clear(glOff.COLOR_BUFFER_BIT);

  if (frames === 0) {
    gl.clearColor(0.1, 0.3, 0.9, 1.0);
  } else {
    gl.clearColor(R, G, B, 1.0);
  }
  gl.clear(gl.COLOR_BUFFER_BIT);

  // Offscreen last as well, so the frame *ends* with a pbuffer current. This is
  // the half that `bypass-probe` cannot reach at all: presentation must
  // re-establish the window surface without any help from the content.
  glOff.clearColor(1.0, 0.0, 0.0, 1.0);
  glOff.clear(glOff.COLOR_BUFFER_BIT);

  frames += 1;
  if (frames === 1 || frames % 60 === 0) {
    console.error(
      `[bypass-multi-probe] painted ${frames} frames, expect rgba(51,204,102,255)`,
    );
  }
  requestAnimationFrame(paint);
}

requestAnimationFrame(paint);
