// Does an image upload leave the content's texture binding where it left it?
//
// The third instance of one defect class. Every per-canvas GL binding is shadowed
// so a redundant call from content can be dropped, which makes the shadow a claim
// about the driver — and the engine's own writes have to maintain it. The texture
// upload path binds its destination, sets `UNPACK_ALIGNMENT`, and finishes by
// binding *zero* rather than restoring what the content had, while telling only
// Skia's tracker about it. The content's next `bindTexture(sameTexture)` then looks
// redundant, never reaches the driver, and everything after it samples or uploads
// against no texture at all.
//
// **The observation needs no shader.** `getParameter(TEXTURE_BINDING_2D)` is
// answered by a real driver query, not from the shadow, so the content can ask
// directly whether the bind it just issued took effect. That turns a question that
// would otherwise need a textured draw and a readback into one integer.
//
// `createImage` is the trigger, but *which* upload path it takes decides whether
// this probe reaches anything. An ordinary image is handed to the upload thread,
// which has a GL context of its own, so it cannot disturb this canvas's bindings —
// a small data-URL image reported `binding_lost=false` for exactly that reason.
// The render-thread paths are: an image too large ever to fit the async per-frame
// byte budget (4 MB on this tier), a compressed texture, an AHB image, and
// `ImagePriority::Critical`. Only the first is reachable from JS on Linux, so
// `oversized.png` is 1200x1200 — 5.49 MB decoded — which fits in 8 KB on disk
// because it is one flat colour, and takes `SyncFallback` deterministically.
const canvas = migo.createCanvas();
const gl = canvas.getContext("webgl");

// A second canvas keeps DrawingBuffer bypass off so the blit path runs, matching
// the other probes. It has to be *used*, not merely referenced: holding it in an
// unused const let it be collected — the run allocates an image every frame, and
// the engine logged canvas_count going 1 -> 2 -> 1 as the finaliser ran, which
// silently moved the probe onto the bypass path. The gate caught that only because
// it reads the path from the engine's own transition log rather than trusting the
// fixture's intent.
const keepBypassOff = migo.createCanvas();
const glKeepAlive = keepBypassOff.getContext("webgl");

const tex = gl.createTexture();
gl.bindTexture(gl.TEXTURE_2D, tex);
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);

let frames = 0;
// Latched, not sampled: the upload lands between frames, so a single frame that
// sees the binding intact proves nothing. Once lost it stays reported.
let bindingWasLost = false;

function paint() {
  // Touch the second canvas so it stays reachable and bypass stays off.
  glKeepAlive.clearColor(0.0, 0.0, 0.0, 1.0);
  glKeepAlive.clear(glKeepAlive.COLOR_BUFFER_BIT);

  // Keep an oversized decode in flight for the whole run.
  const img = migo.createImage();
  img.src = "oversized.png";

  // The bind the shadow may swallow, and a driver query that cannot be fooled by
  // the shadow.
  gl.bindTexture(gl.TEXTURE_2D, tex);
  const bound = gl.getParameter(gl.TEXTURE_BINDING_2D);
  if (!bound) {
    bindingWasLost = true;
  }

  if (bindingWasLost) {
    // Red, matching the other probes' meaning: something the content asked for did
    // not happen.
    gl.clearColor(0.85, 0.1, 0.15, 1.0);
  } else if (frames === 0) {
    // Blue on the first frame only, so a run that stopped presenting reads as
    // frozen rather than as passing.
    gl.clearColor(0.1, 0.3, 0.9, 1.0);
  } else {
    gl.clearColor(0.2, 0.8, 0.4, 1.0);
  }
  gl.clear(gl.COLOR_BUFFER_BIT);

  frames += 1;
  if (frames === 1 || frames % 60 === 0) {
    console.error(
      `[upload-shadow-probe] painted ${frames} frames, binding_lost=${bindingWasLost}, ` +
        `expect rgba(51,204,102,255)`
    );
  }
  requestAnimationFrame(paint);
}

requestAnimationFrame(paint);
