// App lifecycle -> DOM Page Visibility: drive document.hidden /
// document.visibilityState and fire `visibilitychange` from migo.onShow/onHide.
// Run with `node tests/visibility-events.test.mjs`.
//
// HTML5 games pause audio and their loop on `visibilitychange` + `document.hidden`
// when backgrounded. The runtime fires migo.onShow/onHide, and document exposes
// visibilityState/hidden, but nothing wired the two together, so hidden was
// always false and visibilitychange never fired. Device-free, node-testable.

import assert from "node:assert/strict";

let _onShow, _onHide;
globalThis.migo = {
  getWindowInfo: () => ({ windowWidth: 390, windowHeight: 844, screenWidth: 390, screenHeight: 844, pixelRatio: 3 }),
  getSystemInfoSync: () => ({ platform: "android", system: "x", language: "zh-CN", version: "1", screenWidth: 390, screenHeight: 844, pixelRatio: 3 }),
  onWindowResize: () => {},
  createImage: () => ({}), createCanvas: () => ({ getContext: () => ({}) }),
  createInnerAudioContext: () => ({ onCanplay(){}, onPlay(){}, onPause(){}, onEnded(){}, onError(){}, play(){}, pause(){}, seek(){} }),
  getStorageInfoSync: () => ({ keys: [] }), getStorageSync: () => "", setStorageSync: () => {}, removeStorageSync: () => {}, clearStorageSync: () => {},
  request: () => ({ abort() {} }), connectSocket: () => ({ onOpen(){}, onMessage(){}, onClose(){}, onError(){}, send(){}, close(){} }),
  onShow: (cb) => { _onShow = cb; }, onHide: (cb) => { _onHide = cb; },
};

await import("../src/index.js");

let failures = 0;
function check(name, fn) {
  try { fn(); console.log("ok -", name); }
  catch (e) { failures++; console.error("FAIL -", name, "\n   ", e.message); }
}

check("starts visible", () => {
  assert.equal(document.visibilityState, "visible");
  assert.equal(document.hidden, false);
});

check("onHide -> hidden + visibilitychange, state readable in the handler", () => {
  let firedHidden = null;
  const h = () => { firedHidden = document.hidden; };
  document.addEventListener("visibilitychange", h);
  _onHide();
  document.removeEventListener("visibilitychange", h);
  assert.equal(document.hidden, true, "document.hidden must be true when hidden");
  assert.equal(document.visibilityState, "hidden");
  assert.equal(firedHidden, true, "visibilitychange must fire AND document.hidden must already be true inside the handler");
});

check("onShow -> visible + visibilitychange", () => {
  let fired = false;
  const h = () => { fired = true; };
  document.addEventListener("visibilitychange", h);
  _onShow({});
  document.removeEventListener("visibilitychange", h);
  assert.equal(document.hidden, false);
  assert.equal(document.visibilityState, "visible");
  assert.equal(fired, true, "visibilitychange must fire on show");
});

check("document.onvisibilitychange sink also fires", () => {
  let fired = false;
  document.onvisibilitychange = () => { fired = true; };
  _onHide();
  document.onvisibilitychange = null;
  _onShow({}); // restore visible for later tests
  assert.equal(fired, true, "the on-sink must also be called");
});

if (failures > 0) { console.error(`\n${failures} failing`); process.exit(1); }
console.log("\nall visibility-events checks passed");
