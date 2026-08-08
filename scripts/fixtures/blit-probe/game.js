// Blit-path twin of `bypass-probe`, and the positive control for it.
//
// Identical in every respect except the one line that matters: a second canvas
// exists, so `canvas_count == 1` fails and bypass never latches. Both probes
// clear to the same colour and are captured by the same instrument, so a colour
// from this one and nothing from that one localises the fault to the bypass
// presentation path rather than to the fixture, the player, or the capture.
//
// A control is not optional here. "The captured frame is not the clear colour"
// is an absence, and an absence is satisfied by a player that cannot capture, a
// game that never painted, and a PNG decoder that reads the wrong offset —
// none of which say anything about bypass.
const canvas = migo.createCanvas();
const gl = canvas.getContext("webgl");

// Never drawn to, never read. Its whole job is to exist, because existing is
// what `can_bypass_drawing_buffer` counts. The first `createCanvas` is the
// onscreen one; every call after it allocates an offscreen pbuffer canvas.
const _offscreen = migo.createCanvas();

const R = 0.2;
const G = 0.8;
const B = 0.4;

let frames = 0;

function paint() {
  // First frame in a different colour, for the same reason as `bypass-probe`: an
  // engine that presented once and then stopped satisfies "the screen is the clear
  // colour" forever. Blue says frozen; green says live.
  if (frames === 0) {
    gl.clearColor(0.1, 0.3, 0.9, 1.0);
  } else {
    gl.clearColor(R, G, B, 1.0);
  }
  gl.clear(gl.COLOR_BUFFER_BIT);
  frames += 1;
  if (frames === 1 || frames % 60 === 0) {
    console.error(`[blit-probe] painted ${frames} frames, expect rgba(51,204,102,255)`);
  }
  requestAnimationFrame(paint);
}

requestAnimationFrame(paint);
