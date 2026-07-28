// Keyboard probe: the whole screen is one colour, and it changes only when a
// keyboard event arrives. Any pixel difference between two frames is therefore
// attributable to the soft-keyboard round trip -- content asking the host to
// open a keyboard, and the host's events crossing the C ABI back into JS.
//
// The colour encodes which event last arrived, so a screenshot is evidence of
// what reached JS. That matters because a host has no other way to see it:
// engine logs need MIGO_CAPI_LOG set before the engine is created, and a pixel
// needs nothing at all. The value is drawn as text as well, which proves the
// whole string crossed the boundary rather than merely that some event fired.
const canvas = wx.createCanvas();
const ctx = canvas.getContext('2d');

const IDLE = '#c00000';      // red     -- nothing requested yet
const WAITING = '#c0c000';   // yellow  -- showKeyboard called, host has not answered
const INPUT = '#00c000';     // green   -- an input event arrived
const CONFIRM = '#0000c0';   // blue    -- confirm arrived
const COMPLETE = '#c000c0';  // magenta -- complete arrived

let colour = IDLE;
let state = 'idle';
let value = '';
let keyboardHeight = 0;
let inputs = 0;
let frames = 0;
let preedit = '';
let compositions = 0;

function requestKeyboard() {
  state = 'waiting';
  colour = WAITING;
  console.error('[kbprobe] showKeyboard');
  wx.showKeyboard({
    defaultValue: 'seed',
    maxLength: 140,
    multiple: false,
    confirmHold: false,
    confirmType: 'done',
    keyboardType: 'text',
  });
}

function paint() {
  frames++;
  // Ask once, on its own, so a run needs no human at the window: an automated
  // check has to be able to reach the round trip. A touch re-triggers it, which
  // is how a person drives the same path by hand.
  if (frames === 30) {
    requestKeyboard();
  }
  ctx.fillStyle = colour;
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.fillStyle = '#ffffff';
  ctx.font = '28px sans-serif';
  ctx.fillText('state=' + state, 24, 64);
  ctx.fillText('value=' + value, 24, 108);
  ctx.fillText('height=' + keyboardHeight, 24, 152);
  ctx.fillText('inputs=' + inputs, 24, 196);
  // The preedit is what composition adds over the keyboard alone: text that is
  // being typed but not yet committed.
  ctx.fillText('preedit=[' + preedit + ']', 24, 240);
  ctx.fillText('compositions=' + compositions, 24, 284);
  const pad = migo.getGamepads()[0];
  ctx.fillText(pad
    ? 'pad ax0=' + pad.axes[0].toFixed(2) +
      ' b0=' + (pad.buttons[0].pressed ? 'down' : 'up') +
      ' b6=' + pad.buttons[6].value.toFixed(2) +
      '/' + (pad.buttons[6].pressed ? 'down' : 'up')
    : 'pad none', 24, 328);
  requestAnimationFrame(paint);
}
paint();

wx.onTouchStart(requestKeyboard);

wx.onKeyboardInput(function (res) {
  inputs++;
  value = res.value;
  state = 'input';
  colour = INPUT;
  console.error('[kbprobe] input #' + inputs + ' value=' + value);
  // Content correcting the value it was handed is the third verb, and the only
  // thing that exercises it. A real game does this to enforce its own rules.
  wx.updateKeyboard({ value: value });
});

wx.onKeyboardConfirm(function (res) {
  value = res.value;
  state = 'confirm';
  colour = CONFIRM;
  console.error('[kbprobe] confirm value=' + value);
  // Confirm is where a game is done with the field, so this is where hide
  // belongs -- and it is what proves the hide verb reaches the host.
  wx.hideKeyboard();
});

wx.onKeyboardComplete(function (res) {
  value = res.value;
  state = 'complete';
  colour = COMPLETE;
  console.error('[kbprobe] complete value=' + value);
});

wx.onCompositionStart(function (res) {
  compositions++;
  preedit = res.data;
  console.error('[kbprobe] compositionstart data=[' + res.data + ']');
});

wx.onCompositionUpdate(function (res) {
  preedit = res.data;
  console.error('[kbprobe] compositionupdate data=[' + res.data + ']');
});

wx.onCompositionEnd(function (res) {
  // Cleared on end: the committed text arrives as a keyboard input value, and
  // content that kept drawing the preedit would show it twice.
  preedit = '';
  console.error('[kbprobe] compositionend data=[' + res.data + ']');
});

wx.onKeyboardHeightChange(function (res) {
  keyboardHeight = res.height;
  console.error('[kbprobe] height=' + keyboardHeight);
});
