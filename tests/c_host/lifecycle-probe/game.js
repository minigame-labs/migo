// Lifecycle probe: does the engine stop painting while the app is away, and
// paint again when it comes back?
//
// Android is the only place this can be asked. Nothing on a desktop takes the
// application away, so pause, resume and window loss only ever meet the engine
// here -- and until the host learned to stamp a fresh surface generation, the
// second half of the cycle was unreachable even here.
//
// Measured in pixels because a NativeActivity is handed no engine log: the frame
// count at `onHide` and the count at `onShow` are both drawn, so their difference
// is the frames painted while hidden, readable from a screenshot taken after the
// app returns.
//
// The hide and show counts are drawn beside them for the reason any quiescence
// measurement needs a paired liveness reading: "no frames while hidden" is also
// what a probe that never received the callbacks reports, and what an engine that
// stopped painting for good reports. Zero frames between a hide and a show that
// both happened, followed by a count that climbs again, is the only reading that
// means what it says.
const canvas = migo.createCanvas();
const ctx = canvas.getContext('2d');

const AWAKE = '#1b5e9c';   // blue  -- running, no cycle observed yet
const SETTLED = '#00a000'; // green -- came back from a hide having painted nothing
const LEAKED = '#c00000';  // red   -- kept painting while hidden

let frames = 0;
let hides = 0;
let shows = 0;
let framesAtHide = -1;
let framesWhileHidden = -1;

// Reported on both channels on purpose. A screenshot alone cannot distinguish
// "the callback never arrived" from "this frame was painted before it did", and
// reading the pixels of a *counter* was how a run of this probe was first
// misread as a missing callback. `MIGO_CAPI_LOG=info` turns these into logcat
// lines, which timestamps each event against the surface transitions beside it.
migo.onHide(function () {
  hides = hides + 1;
  framesAtHide = frames;
  console.error('[lifecycle-probe] onHide at frame ' + frames);
});

migo.onShow(function () {
  shows = shows + 1;
  // Read on the way back in, not while away: a frame the engine painted after
  // the hide is exactly what this counts, so the window has to close here.
  if (framesAtHide >= 0) {
    framesWhileHidden = frames - framesAtHide;
  }
  console.error('[lifecycle-probe] onShow at frame ' + frames +
    ' painted while hidden ' + framesWhileHidden);
});

function paint() {
  frames = frames + 1;

  // A handful of frames may already be in flight when the hide arrives, so the
  // claim is "it stopped", not "it stopped instantly". Anything beyond a few
  // means the loop ran the whole time the app was away.
  let colour = AWAKE;
  if (framesWhileHidden >= 0) {
    colour = framesWhileHidden <= 5 ? SETTLED : LEAKED;
  }

  ctx.fillStyle = colour;
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  const size = Math.max(12, Math.round(canvas.width / 20));
  ctx.fillStyle = '#ffffff';
  ctx.font = size + 'px sans-serif';
  let y = size * 2;
  ctx.fillText('frames ' + frames, size, y);
  y = y + size * 1.5;
  ctx.fillText('hides ' + hides + ' shows ' + shows, size, y);
  y = y + size * 1.5;
  ctx.fillText('while hidden ' + framesWhileHidden, size, y);

  requestAnimationFrame(paint);
}

paint();
