// Does deduping `glScissor` clip anything it should not?
//
// `glScissor` was deliberately left undeduped for a long time, on the grounds
// that the engine re-points the driver's box behind the state shadow's back —
// `dirty_region::apply_scissor` borrows it for a partial-damage Canvas2D batch,
// and the DrawingBuffer blit toggles the enable bit around a present. A shadow
// that missed either of those would report a hit for a call the driver needed,
// and every later draw would be clipped to whatever box the engine had left
// behind. No GL error, nothing in a log, just wrong pixels.
//
// The dedup landed once both engine paths were routed through the shadow. This
// fixture is the pixel proof, and it is built so that a shadow/driver
// disagreement cannot hide:
//
//   1. Set a scissor box covering the LEFT half, and enable the test.
//   2. Clear the whole surface to RED. Only the left half may go red; the right
//      half keeps whatever was there.
//   3. Re-assert the *same* box — this is the call the dedup will swallow — then
//      set the box to the FULL surface and clear to GREEN.
//
// If the dedup is sound, step 3's full-surface box reaches the driver and the
// frame is flat GREEN. If the engine's borrow left a stale box in the shadow and
// the re-assert in step 3 got eaten, the driver is still clipped to something
// smaller and the RED from step 2 survives on part of the surface — two colours
// instead of one, which the gate reads as a failure rather than having to know
// which colour is wrong.
//
// Step 3's re-assert is what makes this specific. Without it the fixture would
// only prove that a *changing* box works, which no dedup would ever break.

const canvas = migo.createCanvas();
canvas.width = 256;
canvas.height = 256;
const gl = canvas.getContext('webgl');

// rgba(217,26,38,255) — the "something was clipped" colour, matching the other
// probes' failure red.
const RED = [0.85, 0.1, 0.15, 1.0];
// rgba(51,204,102,255) — the verdict colour every probe shares.
const GREEN = [0.2, 0.8, 0.4, 1.0];

// Reported so the gate can tell "painted the right colour" from "painted
// nothing and the capture happens to be that colour". A blank surface can be
// any colour; only a frame count separates the two.
let frames = 0;

function frame() {
  const w = canvas.width;
  const h = canvas.height;
  gl.viewport(0, 0, w, h);

  // 1. Left half only.
  gl.enable(gl.SCISSOR_TEST);
  gl.scissor(0, 0, w / 2, h);

  // 2. Red, clipped to the left half.
  gl.clearColor(RED[0], RED[1], RED[2], RED[3]);
  gl.clear(gl.COLOR_BUFFER_BIT);

  // 3. Re-assert the identical box. This is the call the dedup swallows; if the
  // shadow is stale the driver never hears about it.
  gl.scissor(0, 0, w / 2, h);

  // Then widen to the whole surface and paint green over everything. A driver
  // still holding a narrower box leaves red behind.
  gl.scissor(0, 0, w, h);
  gl.clearColor(GREEN[0], GREEN[1], GREEN[2], GREEN[3]);
  gl.clear(gl.COLOR_BUFFER_BIT);

  gl.disable(gl.SCISSOR_TEST);

  frames++;
  if (frames % 60 === 0) {
    console.log(
      `[scissor-dedup-probe] painted ${frames} frames, expect flat ` +
        `rgba(51,204,102,255); any red means a re-asserted scissor box was ` +
        `deduped away while the driver held a narrower one`,
    );
  }

  requestAnimationFrame(frame);
}

requestAnimationFrame(frame);
