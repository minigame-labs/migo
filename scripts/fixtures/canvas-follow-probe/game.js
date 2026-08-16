// Does a canvas the content never sized still describe the surface after the
// surface was destroyed and recreated at a different size?
//
// The verdict is the whole frame's colour, because a log line can be stale and a
// frame cannot. Flat on purpose: the gate also asserts the surface carries
// exactly one colour, so a partial present cannot read as a pass.
//
// GREY until the window has actually changed size. Green is the expected colour
// AND the pre-transition colour would be too, so an engine that presented one
// frame and then stopped would satisfy "the screen is green" forever with the JS
// loop still running -- the same always-green shape a paired liveness reading
// exists to refuse. Painting the settled state grey means green can only come
// from a frame painted after the surface moved.
const canvas = migo.createCanvas();
const ctx = canvas.getContext('2d');

const WAITING = '#808080'; // grey  -- the surface has not changed yet
const FOLLOWS = '#33cc66'; // green -- canvas describes the new surface
const STALE = '#cc3333';   // red   -- canvas still describes the old one

let initial = null;

function paint() {
  const info = migo.getSystemInfoSync();
  // Integers on both sides: the backing store is a whole number of pixels while
  // `windowWidth` carries the exact surface/ratio, so 1080/2.75 would never
  // compare equal to anything the engine can allocate.
  const windowWidth = Math.round(info.windowWidth);
  const windowHeight = Math.round(info.windowHeight);
  if (initial === null) initial = windowWidth + 'x' + windowHeight;

  let colour = WAITING;
  if (initial !== windowWidth + 'x' + windowHeight) {
    colour = canvas.width === windowWidth && canvas.height === windowHeight ? FOLLOWS : STALE;
  }

  ctx.fillStyle = colour;
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  requestAnimationFrame(paint);
}

paint();
