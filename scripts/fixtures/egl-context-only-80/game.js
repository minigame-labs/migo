// Splitting the ~5 MB an offscreen canvas costs.
//
// `skia-floor-probe-80` measures 80 offscreen canvases at +4.9 MB of Graphics
// each, and the Skia resource-cache ceiling cannot explain it: the per-context
// floor is 64 KiB and the aggregate share at 80 contexts is 1.2 MiB. So the
// cost is fixed overhead, and there are two candidates -- the EGL context that
// `register_offscreen` creates, and the `GrDirectContext` that
// `init_skia_for_canvas` creates.
//
// This fixture isolates the first. `migo.createCanvas()` alone posts
// `RegisterOffscreen`, which creates the EGL context and its pbuffer; the
// DirectContext is only built when `getContext('2d')` is called. So: same 80
// canvases, same sizes, no 2D context, nothing drawn offscreen. Whatever
// Graphics this holds is the EGL half; the difference against
// skia-floor-probe-80 is the Skia half.
//
// The onscreen canvas still draws so the process behaves like a running game
// (and so the frame loop keeps the measurement window alive).
const N = 80;

const canvas = migo.createCanvas();
const ctx = canvas.getContext('2d');

const offscreens = [];
for (let i = 0; i < N; i++) {
  const c = migo.createCanvas();
  c.width = 128;
  c.height = 64;
  offscreens.push(c);       // deliberately no getContext()
}

let frame = 0;
function paint() {
  ctx.fillStyle = '#101014';
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.fillStyle = '#c8c8d0';
  ctx.fillText('egl-context-only ' + N + '  frame ' + frame, 20, 60);
  frame += 1;
  if (frame % 60 === 0) {
    console.error('[egl-context-only-80] frame ' + frame + ', ' + offscreens.length
      + ' offscreen canvases registered without a 2D context');
  }
  requestAnimationFrame(paint);
}
requestAnimationFrame(paint);
