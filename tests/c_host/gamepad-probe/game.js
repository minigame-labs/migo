// Gamepad probe: renders what migo.getGamepads() reports, as pixels.
//
// Called on `migo`: no mini-game platform this engine is compatible with has
// a gamepad API, so 97_migo_namespace.js documents these as beyond the
// common surface (see `_NON_MINIGAME_API`).
// Content that runs through the HTML5 adapter sees them as
// navigator.getGamepads(); content like this probe, which runs directly on
// the runtime, calls them on `migo`.
//
// The JS Gamepad implementation has no unit tests -- it is state held in JS and
// driven from native -- so this content is where it is actually exercised. The
// colour encodes how far the round trip got, and the text carries the values,
// so a screenshot is evidence rather than a log line that could be stale.
const canvas = migo.createCanvas();
const ctx = canvas.getContext('2d');

const NONE = '#c00000';       // red     -- no pad has connected
const CONNECTED = '#c0c000';  // yellow  -- connected, no sample yet
const SAMPLED = '#00c000';    // green   -- axes and buttons arrived
const GONE = '#0000c0';       // blue    -- the pad was withdrawn

let colour = NONE;
let connectEvents = 0;
let disconnectEvents = 0;
let lastId = '';
let summary = '';

migo.onGamepadConnected(function (e) {
  connectEvents++;
  lastId = e.gamepad.id;
  colour = CONNECTED;
  // Read the array lengths here on purpose: content commonly decides its layout
  // in this listener, so they must already be correct rather than arriving with
  // the first sample.
  console.error('[gpprobe] connected id=' + e.gamepad.id +
    ' mapping=' + e.gamepad.mapping +
    ' axes=' + e.gamepad.axes.length +
    ' buttons=' + e.gamepad.buttons.length);
});

migo.onGamepadDisconnected(function (e) {
  disconnectEvents++;
  colour = GONE;
  console.error('[gpprobe] disconnected index=' + e.gamepad.index);
});

function paint() {
  // Polled, as the Web API is: read whatever is current, every frame.
  const pads = migo.getGamepads();
  const pad = pads[0];
  if (pad && pad.timestamp > 0) {
    if (colour === CONNECTED) colour = SAMPLED;
    summary = 'ax0=' + pad.axes[0].toFixed(2) +
      ' ax1=' + pad.axes[1].toFixed(2) +
      ' b0=' + (pad.buttons[0].pressed ? 'down' : 'up') +
      ' b6=' + pad.buttons[6].value.toFixed(2);
  } else if (!pad && disconnectEvents > 0) {
    summary = 'slot empty';
  }

  ctx.fillStyle = colour;
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.fillStyle = '#ffffff';
  ctx.font = '26px sans-serif';
  ctx.fillText('id=' + lastId, 24, 56);
  ctx.fillText('connect=' + connectEvents + ' disconnect=' + disconnectEvents, 24, 96);
  ctx.fillText(summary, 24, 136);
  ctx.fillText('pads=' + migo.getGamepads().length, 24, 176);
  requestAnimationFrame(paint);
}
paint();
