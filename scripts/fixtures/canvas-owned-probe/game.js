// The other half of the same rule: a backing store the content chose must NOT
// move because the surface did.
//
// This one cannot ask whether the engine kept its size -- `canvas.width` is the
// number JS holds, and JS deliberately does not re-read it for a canvas the
// content sized, so it would answer 480 either way. Geometry answers instead.
// The fill covers the canvas's own coordinate space, so if the engine resized the
// backing store underneath it the fill reaches only part of the buffer and the
// rest arrives at the surface as whatever was there -- the "content renders into
// a corner" symptom, as pixels. The gate requires exactly one colour on the
// surface, which is what that breaks.
const canvas = wx.createCanvas();
// A fixed resolution, the Phaser `Scale.NONE` shape, and deliberately neither the
// initial surface nor the resized one so no coincidence can make a moved buffer
// look right.
canvas.width = 480;
canvas.height = 320;
const ctx = canvas.getContext('2d');

const WAITING = '#808080'; // grey  -- the surface has not changed yet
const KEPT = '#33cc66';    // green -- the content's own resolution, filled edge to edge

let initial = null;

function paint() {
  const info = wx.getSystemInfoSync();
  const extent = Math.round(info.windowWidth) + 'x' + Math.round(info.windowHeight);
  if (initial === null) initial = extent;

  ctx.fillStyle = initial === extent ? WAITING : KEPT;
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  requestAnimationFrame(paint);
}

paint();
