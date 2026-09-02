// 80 offscreen canvases with almost no rasterisation, so what remains is the
// per-canvas *fixed* cost of a frame: context binds, Skia flushes, GL state
// invalidation. `multicanvas-80` draws text and is rasterisation-bound on a
// software rasteriser, which hides exactly the effect this is for.
const N = 80;

const canvas = migo.createCanvas();
const ctx = canvas.getContext('2d');

const offscreens = [];
for (let i = 0; i < N; i++) {
  const c = migo.createCanvas();
  c.width = 128;
  c.height = 64;
  offscreens.push({ c, octx: c.getContext('2d') });
}

let frame = 0;
let windowStart = 0;

function paint() {
  for (let i = 0; i < offscreens.length; i++) {
    // One 2x2 rect: enough to make the canvas dirty and force a flush, small
    // enough that fill cost is noise next to the per-canvas overhead.
    const { octx } = offscreens[i];
    octx.fillStyle = (frame + i) % 2 ? '#ff0000' : '#00ff00';
    octx.fillRect(0, 0, 2, 2);
  }
  ctx.fillStyle = '#101014';
  ctx.fillRect(0, 0, 64, 64);

  frame += 1;
  const now = Date.now();
  if (windowStart === 0) {
    windowStart = now;
  } else if (frame % 60 === 0) {
    console.error('fps=' + Math.round((60 * 1000) / (now - windowStart))
      + ' [multicanvas-80-light] frame ' + frame);
    windowStart = now;
  }
  requestAnimationFrame(paint);
}
requestAnimationFrame(paint);
