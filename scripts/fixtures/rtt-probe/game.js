// Render-to-texture across a canvas switch: does the content's own framebuffer
// binding survive the engine re-pointing the driver behind its back?
//
// The engine shadows `bindFramebuffer` to skip redundant driver calls, and keys
// that shadow on the *user-facing* framebuffer id — `null` meaning "the default
// framebuffer". Several engine-internal paths re-point the driver's FRAMEBUFFER
// at the canvas's default framebuffer without telling the shadow: the EGL switch
// in `make_current_needed`, the post-swap restore after the DrawingBuffer blit.
// If the content still has its own FBO in the shadow at that moment, its next
// `bindFramebuffer(sameFbo)` is deduped away and the render-to-texture pass draws
// onto the screen instead.
//
// The probe makes that a colour. Per frame:
//
//   1. bind the default framebuffer, clear GREEN   -> the screen
//   2. bind FBO X                                   -> shadow = X, driver = X
//   3. clear on the *other* canvas                  -> forces an EGL switch away
//      and back, and the switch re-points the driver at the default framebuffer
//   4. bind FBO X again                             -> deduped: shadow already X
//   5. clear RED
//
// Step 5 lands in X when the binding survived, and on the screen when it did
// not. So a captured GREEN frame is the property holding and a RED one is the
// defect — with no dependence on reading back a texture, which would need the
// very framebuffer machinery under test.
//
// The second canvas is WebGL rather than 2D on purpose: a Canvas2D batch is a
// separate phase the render thread may legally reorder ahead of the whole WebGL
// half, which would move the switch out from between steps 2 and 4 and quietly
// turn this into a probe of nothing.
const main = migo.createCanvas();
const gl = main.getContext("webgl");

// Every call after the first `createCanvas` allocates an offscreen pbuffer
// canvas, each with an EGL surface and context of its own. Two canvases also
// keep DrawingBuffer bypass off, so the default framebuffer really is the
// DrawingBuffer and the blit is what carries it to the window.
const other = migo.createCanvas();
const glOther = other.getContext("webgl");

// A *complete* framebuffer, because an incomplete one makes the RED clear a
// no-op and the probe would then report GREEN for a reason unrelated to the
// binding.
const rtt = gl.createFramebuffer();
const tex = gl.createTexture();
gl.bindTexture(gl.TEXTURE_2D, tex);
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 64, 64, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
gl.bindFramebuffer(gl.FRAMEBUFFER, rtt);
gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
const status = gl.checkFramebufferStatus(gl.FRAMEBUFFER);
gl.bindFramebuffer(gl.FRAMEBUFFER, null);

let frames = 0;

function paint() {
  // 1. The baseline the screen must still be showing at the end. The first frame
  //    paints a different colour on purpose: "the screen is green" is an absence
  //    claim, and an engine that stopped presenting satisfies it by leaving frame
  //    one on the surface forever. Blue means frozen, red means the binding was
  //    lost, green means the property holds.
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  if (frames === 0) {
    gl.clearColor(0.1, 0.3, 0.9, 1.0);
  } else {
    gl.clearColor(0.2, 0.8, 0.4, 1.0);
  }
  gl.clear(gl.COLOR_BUFFER_BIT);

  // 2. The content takes its own framebuffer.
  gl.bindFramebuffer(gl.FRAMEBUFFER, rtt);

  // 3. Work on the other canvas, which is what makes the engine leave and
  //    re-enter this canvas's context mid-frame.
  glOther.clearColor(0.0, 0.0, 0.0, 1.0);
  glOther.clear(glOther.COLOR_BUFFER_BIT);

  // 4. The re-bind a real engine issues every frame and this one may skip.
  gl.bindFramebuffer(gl.FRAMEBUFFER, rtt);

  // 5. Off-screen if the binding held; on the screen if it did not.
  gl.clearColor(0.85, 0.1, 0.15, 1.0);
  gl.clear(gl.COLOR_BUFFER_BIT);

  frames += 1;
  if (frames === 1 || frames % 60 === 0) {
    console.error(
      `[rtt-probe] painted ${frames} frames, fbo_status=0x${status.toString(16)}, ` +
        `expect rgba(51,204,102,255) and never rgba(217,26,38,255)`
    );
  }
  requestAnimationFrame(paint);
}

requestAnimationFrame(paint);
