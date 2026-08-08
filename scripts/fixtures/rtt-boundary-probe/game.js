// Render-to-texture across the *frame boundary*: the sibling of `rtt-probe`, and
// the only probe that reaches the post-swap restore.
//
// Both probes ask whether the content's own framebuffer binding survives the
// engine re-pointing the driver behind its back, but they reach different sites,
// and this was found by mutation rather than by design: deleting the shadow record
// from the post-swap restore left `rtt-probe` green, because `rtt-probe`'s first
// framebuffer call each frame binds `null`, which differs from the shadow and is
// issued no matter how stale that shadow is.
//
// To reach the post-swap site the frame's *first* framebuffer call has to be the
// content's own FBO, so it is deduped against the shadow the previous frame left.
// Which means the green baseline must be drawn with no bind at all — legitimate,
// because a frame begins with the default framebuffer bound, and that is precisely
// the guarantee the post-swap restore exists to provide.
//
//   1. clear GREEN with no bind        -> the default framebuffer, i.e. the screen
//   2. bind FBO X                      -> deduped against last frame's shadow
//   3. clear RED                       -> into X if the bind was issued, onto the
//                                         screen if it was deduped away
//
// Frame 1 is the control that comes free: the shadow is empty, so step 2 is issued
// and the frame is green. Every frame after it is the test.
const canvas = migo.createCanvas();
const gl = canvas.getContext("webgl");

// A second canvas keeps DrawingBuffer bypass off, so a blit runs every frame and
// the post-swap restore is on the path. It is never drawn to: unlike `rtt-probe`
// this fixture does not need a mid-frame canvas switch.
const _keepBypassOff = migo.createCanvas();

const rtt = gl.createFramebuffer();
const tex = gl.createTexture();
gl.bindTexture(gl.TEXTURE_2D, tex);
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 64, 64, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
gl.bindFramebuffer(gl.FRAMEBUFFER, rtt);
gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
const status = gl.checkFramebufferStatus(gl.FRAMEBUFFER);
// Back to the default framebuffer so setup leaves the same state a frame ends in.
gl.bindFramebuffer(gl.FRAMEBUFFER, null);

let frames = 0;

function paint() {
  // 1. No bind: a frame starts with the default framebuffer bound, which is the
  //    property the post-swap restore is responsible for.
  //
  //    The first frame paints a *different* colour on purpose. Asserting "the
  //    screen is green" is an absence claim — red never got here — and an engine
  //    that stopped presenting altogether satisfies it by leaving the first frame
  //    on the surface forever. That is not hypothetical: deleting the post-swap
  //    shadow record leaves `draws_to_default_fbo` false, every clear then looks
  //    invisible to damage tracking, and the engine presents exactly one frame for
  //    the whole run. A blue capture says frozen, red says the binding was lost,
  //    green says the property holds.
  if (frames === 0) {
    gl.clearColor(0.1, 0.3, 0.9, 1.0);
  } else {
    gl.clearColor(0.2, 0.8, 0.4, 1.0);
  }
  gl.clear(gl.COLOR_BUFFER_BIT);

  // 2. The frame's first framebuffer call, and the one the shadow can swallow.
  gl.bindFramebuffer(gl.FRAMEBUFFER, rtt);

  // 3. Off-screen if that bind reached the driver; on the screen if it did not.
  gl.clearColor(0.85, 0.1, 0.15, 1.0);
  gl.clear(gl.COLOR_BUFFER_BIT);

  // Deliberately no `bindFramebuffer(null)` here: the frame has to *end* holding
  // the content's own FBO, or the next frame's step 2 has nothing stale to be
  // deduped against.
  frames += 1;
  if (frames === 1 || frames % 60 === 0) {
    console.error(
      `[rtt-boundary-probe] painted ${frames} frames, fbo_status=0x${status.toString(16)}, ` +
        `expect rgba(51,204,102,255) and never rgba(217,26,38,255)`
    );
  }
  requestAnimationFrame(paint);
}

requestAnimationFrame(paint);
