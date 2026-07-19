// Touch probe: the whole screen is one colour, and it changes only when a touch
// arrives. Any pixel difference between two frames is therefore attributable to
// input having crossed the C ABI, the engine, and reached JS -- nothing else in
// this content changes over time.
const canvas = wx.createCanvas();
const ctx = canvas.getContext('2d');
let colour = '#c00000';   // red until touched
let events = 0;

function paint() {
  ctx.fillStyle = colour;
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  requestAnimationFrame(paint);
}
paint();

wx.onTouchStart(function (e) {
  events++;
  colour = '#00c000';     // green while a finger is down
  console.error('[touchprobe] start events=' + events +
                ' x=' + (e && e.touches && e.touches[0] ? e.touches[0].clientX : -1) +
                ' y=' + (e && e.touches && e.touches[0] ? e.touches[0].clientY : -1));
});
wx.onTouchEnd(function () {
  colour = '#0000c0';     // blue after release
  console.error('[touchprobe] end');
});
