// Deleting a per-context GL object has to happen in the context that minted it.
//
// GLES share groups share buffers, textures, renderbuffers, samplers, programs,
// shaders and sync objects. They do **not** share container objects:
// framebuffers, vertex arrays, queries and transform feedbacks are per-context,
// and the same small integer names a different object in every context of the
// group. `glDeleteFramebuffers(n)` issued while the wrong context is current
// therefore either frees nothing -- the name does not exist there, and the object
// leaks with its bookkeeping already discarded -- or it frees *another canvas's*
// framebuffer that holds the same name. Drivers hand these out from 1 upwards per
// context, so a collision is the ordinary case rather than the unlucky one.
//
// The shape here is a game freeing an offscreen render-target pool while the
// onscreen canvas keeps rendering to a target of its own. Three details make it a
// gate rather than a hope:
//
//   * The pool is several framebuffers wide, so the names it frees span whatever
//     the onscreen context has live rather than depending on how one driver
//     numbers them.
//   * The frees are issued in a frame where nothing has named the offscreen
//     canvas, so the onscreen context is still current. `createFramebuffer`
//     carries its canvas and would have switched; that is why the creates and the
//     frees are in different frames.
//   * The onscreen canvas clears the window **first** and runs its
//     render-to-texture pass **second**. Binding a deleted name is an
//     `INVALID_OPERATION` that leaves the previous binding in place, so a
//     destroyed target sends the red pass to the window; had the order been
//     reversed the green clear would have painted over the evidence.
//
// So: green means the onscreen canvas still owns its framebuffer, red means
// another canvas's delete took it, and blue means the engine presented once and
// stopped.
const canvas = migo.createCanvas();
const gl = canvas.getContext("webgl");

// A second canvas with a WebGL context of its own, and therefore a framebuffer
// namespace of its own. Held in a used binding because a canvas nothing uses can
// be collected, which would silently remove the second namespace entirely.
const offscreen = migo.createCanvas();
const glOff = offscreen.getContext("webgl");

const R = 0.2;
const G = 0.8;
const B = 0.4;

// The onscreen canvas's own render target, complete so that a bind of it really
// does divert the clear away from the window.
const rtt = gl.createFramebuffer();
const rttTex = gl.createTexture();
gl.bindTexture(gl.TEXTURE_2D, rttTex);
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 64, 64, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
gl.bindFramebuffer(gl.FRAMEBUFFER, rtt);
gl.framebufferTexture2D(
  gl.FRAMEBUFFER,
  gl.COLOR_ATTACHMENT0,
  gl.TEXTURE_2D,
  rttTex,
  0,
);
gl.bindFramebuffer(gl.FRAMEBUFFER, null);

const POOL = 8;
let pool = [];
let frames = 0;

function paint() {
  // The window first, so a diverted red pass below is what survives to the swap.
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  if (frames === 0) {
    gl.clearColor(0.1, 0.3, 0.9, 1.0);
  } else {
    gl.clearColor(R, G, B, 1.0);
  }
  gl.clear(gl.COLOR_BUFFER_BIT);

  // The onscreen canvas's own render-to-texture pass. Red must never reach the
  // window, which it only can if `rtt` stopped being a framebuffer.
  gl.bindFramebuffer(gl.FRAMEBUFFER, rtt);
  gl.clearColor(1.0, 0.0, 0.0, 1.0);
  gl.clear(gl.COLOR_BUFFER_BIT);

  if (frames === 0) {
    for (let i = 0; i < POOL; i += 1) {
      pool.push(glOff.createFramebuffer());
    }
  } else if (frames === 1) {
    // Nothing in this frame has named the offscreen canvas, so the onscreen
    // context is the current one when these frees are dispatched.
    for (const fb of pool) {
      glOff.deleteFramebuffer(fb);
    }
    pool = [];
    console.error(`[fbo-owner-probe] freed ${POOL} offscreen framebuffers`);
  }

  frames += 1;
  if (frames === 1 || frames % 60 === 0) {
    console.error(
      `[fbo-owner-probe] painted ${frames} frames, expect rgba(51,204,102,255)`,
    );
  }
  requestAnimationFrame(paint);
}

requestAnimationFrame(paint);
