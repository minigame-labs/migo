// Desktop pointer: touch + W3C compatibility-mouse events, in the adapter.
// Run with `node tests/pointer-compat.test.mjs`.
//
// Contract:
//  - Touch DOM events are dispatched and support preventDefault (real Event).
//  - Mouse/wheel are forwarded to DOM (previously not forwarded at all).
//  - A tap that maps to BOTH touch and mouse fires the compat mouse events
//    ONLY IF the touch was not preventDefault()ed -- and always AFTER touch.
//  - A pure mouse interaction (no concurrent touch) always fires mouse.
//  - Robust to host send order: touch-before-mouse and mouse-before-touch both
//    behave identically (the embedded layer delivers mouse synchronously and
//    touch on a microtask; the adapter must not depend on which was enqueued
//    first).

import assert from "node:assert/strict";

// ---- Fake migo runtime: capture the callbacks the adapter registers. --------
let _touchStart, _touchEnd, _mouseDown, _mouseUp, _wheel;
globalThis.migo = {
  getWindowInfo: () => ({ windowWidth: 390, windowHeight: 844, screenWidth: 390, screenHeight: 844, pixelRatio: 3 }),
  getSystemInfoSync: () => ({ platform: "android", system: "x", language: "en", version: "1", screenWidth: 390, screenHeight: 844, pixelRatio: 3 }),
  onWindowResize: () => {},
  createImage: () => ({}), createCanvas: () => ({ getContext: () => ({}) }),
  createInnerAudioContext: () => ({ onCanplay(){}, onPlay(){}, onPause(){}, onEnded(){}, onError(){}, play(){}, pause(){}, seek(){} }),
  getStorageInfoSync: () => ({ keys: [] }), getStorageSync: () => "", setStorageSync: () => {}, removeStorageSync: () => {}, clearStorageSync: () => {},
  request: () => ({ abort() {} }), connectSocket: () => ({ onOpen(){}, onMessage(){}, onClose(){}, onError(){}, send(){}, close(){} }),

  onTouchStart: (cb) => { _touchStart = cb; }, onTouchMove: () => {},
  onTouchEnd: (cb) => { _touchEnd = cb; }, onTouchCancel: () => {},
  onMouseDown: (cb) => { _mouseDown = cb; }, onMouseMove: () => {},
  onMouseUp: (cb) => { _mouseUp = cb; }, onWheel: (cb) => { _wheel = cb; },
};

await import("../src/index.js");

// ---- Helpers ----------------------------------------------------------------
const flush = () => new Promise((r) => setTimeout(r, 0)); // drains micro + macro tasks

// `document` delegates to a private EventTarget, so listeners can only be
// cleared by removeEventListener. Track everything we add so reset() can undo
// it -- otherwise a preventDefault listener from one test poisons the next.
const _tracked = [];
function on(type, fn) { document.addEventListener(type, fn); _tracked.push([type, fn]); return fn; }
function untrackAll() { for (const [t, f] of _tracked.splice(0)) document.removeEventListener(t, f); }

function recordOn() {
  const log = [];
  for (const t of ["touchstart", "touchend", "mousedown", "mouseup", "click", "wheel"]) {
    on(t, () => log.push(t));
  }
  return log;
}

const touchStartEvt = () => ({ type: "touchstart", touches: [{ identifier: 0, clientX: 5, clientY: 6 }], changedTouches: [{ identifier: 0, clientX: 5, clientY: 6 }], timeStamp: 1 });
const touchEndEvt = () => ({ type: "touchend", touches: [], changedTouches: [{ identifier: 0, clientX: 5, clientY: 6 }], timeStamp: 2 });
const mouseArgs = () => ({ x: 5, y: 6, button: 0, timeStamp: 1 });

let failures = 0;
// Isolate tests: the adapter keeps module-level compat state and dispatches to
// the shared `document`, so clear listeners and end any dangling touch
// interaction (which clears the adapter's `_touchCompat` on its macrotask).
async function reset() {
  untrackAll();
  _touchEnd(touchEndEvt());
  await flush();
}
function check(name, fn) {
  return reset().then(fn).then(
    () => console.log("ok -", name),
    (e) => { failures++; console.error("FAIL -", name, "\n   ", e.message); },
  );
}

// ---- 1. Touch DOM events dispatch and support preventDefault ----------------
await check("touch dispatches and preventDefault is observable", async () => {
  const log = recordOn();
  let prevented = null;
  on("touchstart", (e) => { e.preventDefault(); prevented = e.defaultPrevented; });
  _touchStart(touchStartEvt());
  await flush();
  assert.ok(log.includes("touchstart"), "touchstart must dispatch");
  assert.equal(prevented, true, "touch event must support preventDefault -> defaultPrevented");
});

// ---- 2. Pure mouse (no touch) always forwards to DOM ------------------------
await check("mouse-only forwards mousedown/mouseup to DOM", async () => {
  const log = recordOn();
  _mouseDown(mouseArgs());
  _mouseUp(mouseArgs());
  await flush();
  assert.deepEqual(log, ["mousedown", "mouseup", "click"], "mouse-only must reach DOM (with click after mouseup, per DOM)");
});

// ---- 3. Tap = touch + mouse, no preventDefault: both, touch BEFORE mouse ----
async function tap({ prevent, order }) {
  const log = recordOn();
  if (prevent) on("touchstart", (e) => e.preventDefault());
  const deliverTouch = () => { _touchStart(touchStartEvt()); _touchEnd(touchEndEvt()); };
  const deliverMouse = () => { _mouseDown(mouseArgs()); _mouseUp(mouseArgs()); };
  if (order === "touch-first") { queueMicrotask(deliverTouch); deliverMouse(); }
  else { deliverMouse(); queueMicrotask(deliverTouch); }
  await flush();
  return log;
}

for (const order of ["touch-first", "mouse-first"]) {
  await check(`tap without preventDefault fires touch then compat-mouse (${order})`, async () => {
    const log = await tap({ prevent: false, order });
    assert.deepEqual(log, ["touchstart", "touchend", "mousedown", "mouseup", "click"],
      "touch first, then compat mouse (mousedown/up/click)");
  });

  // ---- 4. Core invariant: preventDefault on touch suppresses compat-mouse ---
  await check(`tap with preventDefault suppresses compat-mouse (${order})`, async () => {
    const log = await tap({ prevent: true, order });
    assert.deepEqual(log, ["touchstart", "touchend"],
      "preventDefault on touchstart must suppress mousedown/mouseup/click");
  });
}

// ---- 5. Wheel forwards to DOM ----------------------------------------------
await check("wheel forwards to DOM wheel", async () => {
  const log = recordOn();
  _wheel({ deltaX: 0, deltaY: 10, deltaZ: 0, deltaMode: 0, timeStamp: 1 });
  await flush();
  assert.ok(log.includes("wheel"), "wheel must reach DOM");
});

if (failures > 0) { console.error(`\n${failures} failing`); process.exit(1); }
console.log("\nall pointer-compat checks passed");
