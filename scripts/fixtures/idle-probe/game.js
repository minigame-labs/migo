// Idle-quiescence probe game: paint exactly one frame, then stop asking for
// frames. A demand-driven engine must go completely quiet after this; a
// fixed-interval ticker keeps waking at the frame rate forever.
const canvas = migo.createCanvas();
const ctx = canvas.getContext("2d");

let painted = 0;

function paintOnce(ts) {
  ctx.fillStyle = "#1a3d5c";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.fillStyle = "#ffcc00";
  ctx.fillRect(60, 60, 240, 240);
  painted += 1;
  console.error(`[idle-probe] painted frame ${painted} at ts=${Math.round(ts)}`);
  // Two frames, then never again: the second proves the loop can still be
  // driven after the first, so a silent engine afterwards is quiescence rather
  // than a stall.
  if (painted < 2) {
    requestAnimationFrame(paintOnce);
  } else {
    console.error("[idle-probe] no further frames requested; engine must idle");
  }
}

requestAnimationFrame(paintOnce);
